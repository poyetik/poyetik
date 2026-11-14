//! Tests of the steps of the check: commands, actions, their verdicts, and the order they run in.

use core::num::NonZeroUsize;
use std::path::Path;

use crate::tests::{
    CHANGES_MODE, FAKE_JOBS, HISTORY_LINES, LICENSE_LINES, METADATA, PACKAGE, PINS_LISTED, cite,
    command_lines, failed, fixture_metadata, one_mutant, passing, run_cli, targets_metadata,
    test_code_gate, test_data_gate, untested_targets_gate, unviable_names_gate,
};
use crate::{
    Action, CARGO_TOML, CHANGELOG, Cmd, Coverage, Dir, EQUIVALENT_REVIEWED, Extent, Failure,
    MUTANTS_OUT, Mode, PACKAGES, Step, UNSAFE_FILES, UNVIABLE_REPORT, UNVIABLE_REVIEWED,
    check_steps, package_jsons, run_steps,
};

/// What each step of the default check does, in order, as `command_lines` says it.
const DEFAULT_PIPELINE: &[&str] = &[
    "preflight",
    "tree recorded",
    "Root: pnpm install --frozen-lockfile",
    "fresh tooling/config/coverage tooling/config/reports/mutation tooling/oracle/coverage tooling/oracle/reports/mutation bindings/audio-wasm/coverage bindings/audio-wasm/reports/mutation apps/playground/coverage apps/playground/reports/mutation apps/roadmap/coverage apps/roadmap/reports/mutation",
    "fresh bindings/audio-wasm/dist",
    "equivalent gate",
    "bypass gate",
    "unsafe gate",
    "cargo config gate",
    "test code gate",
    "test data gate",
    "untested targets gate",
    "unviable names gate",
    "conformance cargo run --package audio-cli --locked --quiet -- check conformance --spent target/xtask/spent-algorithms.json",
    "Root: cargo deny --locked check",
    "dependency gate",
    "pin gate",
    "library gate",
    "part license gate",
    "math layer gate",
    "supply chain gate",
    "membership gate",
    "readme gate",
    "citation gate",
    "vocabulary gate",
    "version gate",
    "history gate",
    "workflow gate",
    "parts gate",
    "Root: pnpm audit --audit-level=low",
    "license gate",
    "stryker config gate",
    "Root: cargo fmt --all --check",
    "Root: cargo clippy --workspace --all-targets --locked",
    "float method gate",
    "Root: cargo doc --workspace --no-deps --document-private-items --locked",
    "Root: cargo test --workspace --locked -- --include-ignored",
    "loaded files gate",
    "fused build",
    "Root: cargo build --package audio-core --package audio-boxes --package audio-patch --package audio-hash --locked --target aarch64-apple-darwin",
    "Root: cargo build --package audio-core --package audio-boxes --package audio-patch --package audio-hash --locked --target x86_64-unknown-linux-gnu",
    "Root: cargo build --package audio-core --package audio-boxes --package audio-patch --package audio-hash --locked --target wasm32-unknown-unknown",
    "Root: cargo build --package audio-core --package audio-boxes --package audio-patch --package audio-hash --locked --target thumbv7em-none-eabihf",
    "Root: cargo clippy --package audio-fpenv --package audio-cli --all-targets --locked --target aarch64-apple-darwin",
    "Root: cargo clippy --package audio-fpenv --package audio-cli --all-targets --locked --target x86_64-unknown-linux-gnu",
    "Root: cargo build --lib --package audio-fpenv --package audio-cli --locked --target aarch64-apple-darwin",
    "Root: cargo build --lib --package audio-fpenv --package audio-cli --locked --target x86_64-unknown-linux-gnu",
    "fresh target/coverage",
    "coverage locality",
    "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/audio-core/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/audio-core.json -- --include-ignored",
    "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/audio-boxes/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/audio-boxes.json -- --include-ignored",
    "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/audio-patch/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/audio-patch.json -- --include-ignored",
    "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/audio-hash/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/audio-hash.json -- --include-ignored",
    "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/alloc-counter/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/alloc-counter.json -- --include-ignored",
    "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/audio-fpenv/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/audio-fpenv.json -- --include-ignored",
    "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/audio-battery/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/audio-battery.json -- --include-ignored",
    "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/audio-testkit/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/audio-testkit.json -- --include-ignored",
    "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/audio-cli/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/audio-cli.json -- --include-ignored",
    "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path bindings/audio-wasm/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/audio-wasm.json -- --include-ignored",
    "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/xtask/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/xtask.json -- --include-ignored",
    "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/audio-cli/Cargo.toml --locked --branch --json --summary-only --output-path target/coverage/audio-cli-main.json -- --include-ignored",
    "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/xtask/Cargo.toml --locked --branch --json --summary-only --output-path target/coverage/xtask-main.json -- --include-ignored",
    concat!(
        "Root: cargo +nightly-2026-11-01 mi",
        "ri test --package alloc-counter --package audio-wasm --locked -- --include-ignored"
    ),
    "Package(\"tooling/config\"): pnpm run check",
    "Package(\"tooling/config\"): pnpm exec stryker run stryker.config.json --incremental --incrementalFile reports/stryker-incremental.json",
    "typescript reports gate of tooling/config",
    "typescript scope gate of tooling/config",
    "fresh target/conformance",
    "Root: cargo run --package audio-cli --locked --quiet -- export conformance target/conformance",
    "Package(\"tooling/oracle\"): pnpm run check",
    "Package(\"tooling/oracle\"): pnpm exec stryker run stryker.config.json --incremental --incrementalFile reports/stryker-incremental.json",
    "typescript reports gate of tooling/oracle",
    "typescript scope gate of tooling/oracle",
    "Package(\"bindings/audio-wasm\"): pnpm run check",
    "Package(\"bindings/audio-wasm\"): pnpm exec stryker run stryker.config.json --incremental --incrementalFile reports/stryker-incremental.json",
    "typescript reports gate of bindings/audio-wasm",
    "typescript scope gate of bindings/audio-wasm",
    "Package(\"bindings/audio-wasm\"): pnpm run build",
    "Package(\"apps/playground\"): pnpm run check",
    "Package(\"apps/playground\"): pnpm exec stryker run stryker.config.json --incremental --incrementalFile reports/stryker-incremental.json",
    "typescript reports gate of apps/playground",
    "typescript scope gate of apps/playground",
    "Package(\"apps/roadmap\"): pnpm run check",
    "Package(\"apps/roadmap\"): pnpm exec stryker run stryker.config.json --incremental --incrementalFile reports/stryker-incremental.json",
    "typescript reports gate of apps/roadmap",
    "typescript scope gate of apps/roadmap",
    "fresh mutants.out",
    "rust mutants of the changes (3 jobs)",
    "unviable gate (Partial)",
    "unviable reviews current",
    "mutation proof recorded",
    "tree unchanged",
];

#[test]
fn lists_every_check_in_order() {
    assert_eq!(
        command_lines(&check_steps(CHANGES_MODE)),
        DEFAULT_PIPELINE,
        "the default check pipeline"
    );
}

/// The name of each step of the default check, in order.
const DEFAULT_CHECK_NAMES: &[&str] = &[
    "Tools and platform",
    "Working tree recorded",
    "JavaScript dependencies",
    "Fresh TypeScript report directories",
    "Fresh TypeScript build outputs",
    "TypeScript equivalent mutants reviewed",
    "Every bypass of coverage or mutation testing reviewed",
    "Unsafe code confined to the audited files",
    "Cargo configuration: no build flag, a locked alias",
    "Test code only in tests/ and declared *_tests.rs files",
    "The tests of each crate read only its declared test data",
    "Tests turned off only where no test can hide, doc tests kept",
    "Every function a review of an unviable mutant names exists",
    "Conformance (native)",
    "Rust supply chain",
    "Every direct dependency reviewed",
    "Bit-defining dependencies pinned to one exact version",
    "The audio library depends only on its own crates, its tests also on its test crates; every crate only tests use is shared test code",
    "Every part under the license of its folder, no open part depending on an app",
    "Box arithmetic only through the math layer: libm in audio-core and audio-testkit only, no raw float in box code, no iterator reduction outside the math layer",
    "Supply-chain configuration as xtask defines it",
    "Packages and crates known to the checks",
    "Every part described in a README",
    "Every roadmap step cited exists on the board",
    "The words of the glossary, not their synonyms",
    "One version everywhere, changelog well formed, released sections as on main",
    "Commit messages, and a changelog entry for each change of behavior",
    "CI runs exactly the workflow xtask defines",
    "The parts of CI run every check of the full check once",
    "JavaScript vulnerabilities",
    "JavaScript licenses",
    "Stryker configurations hide no mutant",
    "Rust formatting",
    "Rust lints",
    "Float methods of the pinned toolchain banned or reviewed as deterministic",
    "Rust documentation",
    "Rust tests",
    "Files rustc loads: tracked, in their crate, those of tests alone named as test code",
    "Boxes unchanged under the math layer's fused multiply-add option (a test build)",
    "Engine portability (aarch64-apple-darwin)",
    "Engine portability (x86_64-unknown-linux-gnu)",
    "Engine portability (wasm32-unknown-unknown)",
    "Engine portability (thumbv7em-none-eabihf)",
    "Native shell lints (aarch64-apple-darwin)",
    "Native shell lints (x86_64-unknown-linux-gnu)",
    "Native code of audio-cli and audio-fpenv built, inline assembly included (aarch64-apple-darwin)",
    "Native code of audio-cli and audio-fpenv built, inline assembly included (x86_64-unknown-linux-gnu)",
    "Fresh Rust coverage reports",
    "Rust coverage counts a crate's own unit tests only",
    "Rust coverage of audio-core: 100% by its own unit tests",
    "Rust coverage of audio-boxes: 100% by its own unit tests",
    "Rust coverage of audio-patch: 100% by its own unit tests",
    "Rust coverage of audio-hash: 100% by its own unit tests",
    "Rust coverage of alloc-counter: 100% by its own unit tests",
    "Rust coverage of audio-fpenv: 100% by its own unit tests",
    "Rust coverage of audio-battery: 100% by its own unit tests",
    "Rust coverage of audio-testkit: 100% by its own unit tests",
    "Rust coverage of audio-cli: 100% by its own unit tests",
    "Rust coverage of audio-wasm: 100% by its own unit tests",
    "Rust coverage of xtask: 100% by its own unit tests",
    "Rust coverage of the audio-cli binary: 100% by its tests",
    "Rust coverage of the xtask binary: 100% by its tests",
    "Unsafe code under Miri",
    "Shared TypeScript policy (each rule proven to reject what it forbids)",
    "Mutation testing of tooling/config (what changed since the last run)",
    "TypeScript coverage and mutation reports of tooling/config",
    "Every TypeScript source of tooling/config measured and mutated, or reviewed",
    "Fresh conformance export directory",
    "Conformance export (native WAV files)",
    "Conformance oracle (independent, TypeScript)",
    "Mutation testing of tooling/oracle (what changed since the last run)",
    "TypeScript coverage and mutation reports of tooling/oracle",
    "Every TypeScript source of tooling/oracle measured and mutated, or reviewed",
    "Engine WASM checks",
    "Mutation testing of bindings/audio-wasm (what changed since the last run)",
    "TypeScript coverage and mutation reports of bindings/audio-wasm",
    "Every TypeScript source of bindings/audio-wasm measured and mutated, or reviewed",
    "Engine WASM build, for the packages that use it",
    "Playground checks",
    "Mutation testing of apps/playground (what changed since the last run)",
    "TypeScript coverage and mutation reports of apps/playground",
    "Every TypeScript source of apps/playground measured and mutated, or reviewed",
    "Roadmap checks",
    "Mutation testing of apps/roadmap (what changed since the last run)",
    "TypeScript coverage and mutation reports of apps/roadmap",
    "Every TypeScript source of apps/roadmap measured and mutated, or reviewed",
    "Fresh Rust mutation report",
    "Rust mutation testing of the changes",
    "Rust unviable mutants reviewed",
    "Rust unviable mutant reviews all current",
    "Rust mutation proof recorded for the next default check",
    "Working tree unchanged by the checks",
];

#[test]
fn names_every_check_in_order() {
    let steps = check_steps(CHANGES_MODE);
    assert_eq!(
        steps
            .iter()
            .map(|step| step.name.as_str())
            .collect::<Vec<_>>(),
        DEFAULT_CHECK_NAMES,
        "step names"
    );
}

/// The steps of the default check in the order of the full check, without the two it adds
/// after its unviable gate.
fn default_steps_as_full() -> Vec<Step> {
    let mut changes: Vec<Step> = crate::steps::full_order(CHANGES_MODE)
        .into_iter()
        .map(|(_, step)| step)
        .collect();
    let current = changes
        .iter()
        .position(|step| step.action == Action::UnviableReviewsCurrent)
        .expect("the default check refuses stale reviews");
    let before = changes
        .get(current.saturating_sub(1))
        .map(|step| step.action.clone());
    let added: Vec<Action> = before
        .into_iter()
        .chain(
            changes
                .drain(current..=current.saturating_add(1))
                .map(|step| step.action),
        )
        .collect();
    assert_eq!(
        added,
        [
            Action::UnviableGate(Extent::Partial),
            Action::UnviableReviewsCurrent,
            Action::MutationProofRecorded
        ],
        "after the unviable gate"
    );
    changes
}

#[test]
fn the_default_check_runs_the_steps_of_its_full_order_cheapest_first() {
    let sorted = |steps: &[Step]| {
        let mut lines = command_lines(steps);
        lines.sort();
        lines
    };
    let unordered: Vec<Step> = crate::steps::full_order(CHANGES_MODE)
        .into_iter()
        .map(|(_, step)| step)
        .collect();
    let default = check_steps(CHANGES_MODE);
    assert_ne!(
        command_lines(&default),
        command_lines(&unordered),
        "reordered"
    );
    assert_eq!(
        sorted(&default),
        sorted(&unordered),
        "the same steps, each once"
    );
    let jobs = NonZeroUsize::new(3).expect("three is not zero");
    let full: Vec<Step> = crate::steps::full_order(Mode::Full { jobs })
        .into_iter()
        .map(|(_, step)| step)
        .collect();
    assert_eq!(
        check_steps(Mode::Full { jobs }),
        full,
        "the full check keeps its order"
    );
}

#[test]
fn prepares_what_a_part_needs_before_its_first_step_in_the_default_check() {
    let roles = crate::steps::roles(CHANGES_MODE);
    let mut checked = 0_usize;
    for (at, (role, step)) in roles.iter().enumerate() {
        let crate::parts::Role::Proof(part) = role else {
            continue;
        };
        for setup in part.needs() {
            let preparing: Vec<usize> = roles
                .iter()
                .enumerate()
                .filter(|(_, (other, _))| *other == crate::parts::Role::Setup(setup))
                .map(|(index, _)| index)
                .collect();
            assert!(
                !preparing.is_empty() && preparing.iter().all(|index| *index < at),
                "{setup:?} prepared before {}",
                step.name
            );
            checked = checked.saturating_add(1);
        }
    }
    assert!(checked > 0, "some part needs a preparation");
}

#[test]
fn the_full_check_differs_only_by_its_mutation_testing() {
    let jobs = NonZeroUsize::new(3).expect("three is not zero");
    // The default check refuses the stale reviews its partial run cannot see, then
    // records the proof of its mutation testing, which the full check never reads nor
    // writes: two steps more, after the unviable gate.
    let changes = default_steps_as_full();
    let full = check_steps(Mode::Full { jobs });
    assert!(
        !full.iter().any(|step| matches!(
            step.action,
            Action::MutationProofRecorded | Action::UnviableReviewsCurrent
        )),
        "the full check records nothing, and its complete run sees stale reviews"
    );
    assert_eq!(changes.len(), full.len(), "the same number of steps");
    let differences: Vec<(String, String)> = command_lines(&changes)
        .into_iter()
        .zip(command_lines(&full))
        .filter(|(default, every)| default != every)
        .collect();
    let stryker = |dir: &str, force: &str| {
        (
            format!(
                "Package(\"{dir}\"): pnpm exec stryker run stryker.config.json --incremental --incrementalFile reports/stryker-incremental.json"
            ),
            format!(
                "Package(\"{dir}\"): pnpm exec stryker run stryker.config.json --incremental --incrementalFile reports/stryker-incremental.json{force}"
            ),
        )
    };
    let mut expected = vec![
        (
            "rust mutants of the changes (3 jobs)".to_owned(),
            "Root: cargo mutants --no-config --output=. --cap-lints=true --gitignore=true --cargo-arg=--locked --cargo-arg=--config=profile.test.debug=false --workspace --jobs 3 --timeout-multiplier 15 --minimum-test-timeout 60 -- -- --include-ignored".to_owned(),
        ),
        (
            "unviable gate (Partial)".to_owned(),
            "unviable gate (Complete)".to_owned(),
        ),
    ];
    expected.extend(
        PACKAGES
            .iter()
            .map(|package| stryker(package.dir, " --force")),
    );
    assert_eq!(differences, expected, "only the mutation testing differs");
    let names: Vec<(&str, &str)> = changes
        .iter()
        .zip(&full)
        .map(|(default, every)| (default.name.as_str(), every.name.as_str()))
        .filter(|(default, every)| default != every)
        .collect();
    assert_eq!(
        names,
        vec![
            (
                "Rust mutation testing of the changes",
                "Rust mutation testing"
            ),
            (
                "Mutation testing of tooling/config (what changed since the last run)",
                "Mutation testing of tooling/config (every mutant)"
            ),
            (
                "Mutation testing of tooling/oracle (what changed since the last run)",
                "Mutation testing of tooling/oracle (every mutant)"
            ),
            (
                "Mutation testing of bindings/audio-wasm (what changed since the last run)",
                "Mutation testing of bindings/audio-wasm (every mutant)"
            ),
            (
                "Mutation testing of apps/playground (what changed since the last run)",
                "Mutation testing of apps/playground (every mutant)"
            ),
            (
                "Mutation testing of apps/roadmap (what changed since the last run)",
                "Mutation testing of apps/roadmap (every mutant)"
            ),
        ],
        "the steps that differ say how much they mutate"
    );
    assert_eq!(
        MUTANTS_OUT, "mutants.out",
        "the directory cargo-mutants writes"
    );
    assert!(
        UNVIABLE_REPORT.starts_with(&format!("{MUTANTS_OUT}/")),
        "the unviable report is in it"
    );
}

/// The test timeout scales with the jobs that share the processors: five times the
/// unmutated tests' time and at least twenty seconds per job (cargo-mutants' defaults,
/// written here as literals), never cargo-mutants' own unscaled timeout.
#[test]
fn scales_the_mutant_timeout_with_the_jobs() {
    for (jobs, multiplier, floor) in [(1, "5", "20"), (2, "10", "40"), (8, "40", "160")] {
        assert_eq!(
            crate::mutant_timeout_args(NonZeroUsize::new(jobs).expect("not zero")),
            [
                "--jobs".to_owned(),
                jobs.to_string(),
                "--timeout-multiplier".to_owned(),
                multiplier.to_owned(),
                "--minimum-test-timeout".to_owned(),
                floor.to_owned(),
            ],
            "{jobs} jobs"
        );
    }
}

#[test]
fn resolves_directories() {
    let root = Path::new("/repo");
    assert_eq!(Dir::Root.under(root), Path::new("/repo"), "root");
    assert_eq!(
        Dir::Package("apps/roadmap").under(root),
        Path::new("/repo/apps/roadmap"),
        "a package"
    );
}

#[test]
fn runs_every_step_and_reports_success() {
    let mut fake = passing();
    let mut out = Vec::new();
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut fake, &mut out),
        Ok(()),
        "all green"
    );
    let (tools, commands, emptied, listed, captured, written) = (10, 58, 14, 27, 55, 4);
    let read = 9
        + 2
        + 13
        + 7
        + 14
        + 3
        + 7
        + 21
        + 3
        + 1
        + 7
        + 3
        + 12
        + 10
        + 21
        + 8
        + 2
        + 12
        + 40
        + 20
        + 3
        + 3 * 9
        + 3 * 12
        + 1
        + 1
        + 6
        + 3
        + 91;
    assert_eq!(
        fake.ran.len(),
        tools + commands + emptied + listed + captured + written + read,
        "the preflight asks ten tools; 58 commands run (formatting, lints, documentation, \
         tests, four portability builds, the native shell's lints and the build of its libraries for its two architectures, fourteen coverage measurements each after emptying \
         the coverage build, the mutants of the changes, Miri, the \
         conformance check and export, deny, the JavaScript install, audit, the engine build and five package \
         checks each followed by its Stryker run); fourteen directories are emptied; git lists \
         fourteen kinds of files (the box sources, the text files that cite the roadmap and the npm configuration files among them), the Rust files again for the test code, test data, untested targets, unviable names and loaded files gates, the TypeScript ones again for the reports and scope gates of \
         each package after the first; the licenses twice, the history of main and the history three times, the state \
         of the tree (before, for the proof record and after) six times, the nightly's version and sysroot, the Cargo metadata (for the untested targets, dependency, pin, library, part license and math layer gates), the output of the fused test build and of the nine builds of the loaded files gate, the merge base, the merge base with main three times and the record of the versions spent and the changelog listed there, the changed paths, the changes, the \
         new Rust files and the list of every mutant once, git's common directory twice, the tree checked out, \
         cargo-mutants' version, rustc's host and build, the linker driver's and git's versions, the status for a push and the hidden files and the tracked files (for their filters) for the mutation step and the proof record; the spent record, the measured base, the diff and the \
         proof record are written once each; the proof record, the measured base and the \
         toolchain file are read for it; and the gates read the nine dep-info files of the loaded files gate, the nine fake test data readers for the unsafe, bypass and vocabulary gates, every Rust file (twelve) for the test code, test data, untested targets and unviable names gates and the reviews of unviable mutants for the last, .node-version and the toolchain file for the preflight, the toolchain file, the four float sources and the float macros of the nightly and clippy.toml for the float gate, fourteen coverage reports, \
         the unviable report and reviews, the register of the pin gate, the 91 text files of the citation gate (the board and the spent record, which the fake lists as tracked, among them), the reviews again against the list of every mutant, six TypeScript sources and their reviews, the nine \
         sources, the twelve Cargo manifests and the register of the bypass gate, the three unsafe files, the Cargo \
         configuration, the six package manifests and the register of the dependency gate, the six package manifests and the three license files of the part license gate, the four \
         supply-chain files, twelve crate manifests, ten READMEs, the changelog, ten READMEs, three Rust and six TypeScript sources and the vocabulary of the vocabulary gate, eight version files, the workflow and \
         .node-version, five Stryker configurations and the six package manifests, and for each of the five \
         packages, the six TypeScript sources and two reports of its reports gate, then its \
         Stryker configuration, coverage summary, mutation report and the register of its \
         scope gate"
    );
    for path in [CARGO_TOML.to_owned(), CHANGELOG.to_owned()]
        .into_iter()
        .chain(package_jsons())
    {
        assert!(fake.ran.contains(&format!("read {path}")), "{path} read");
    }
    assert!(
        fake.ran.contains(&format!("read {UNVIABLE_REVIEWED}")),
        "reviews read"
    );
    assert!(
        fake.ran
            .contains(&"read target/coverage/xtask-main.json".to_owned()),
        "coverage reports read"
    );
    let text = String::from_utf8(out).expect("utf-8 output");
    assert!(
        text.starts_with("▶ [1/92] Tools and platform\n"),
        "numbered progress: {text}"
    );
    assert!(
        text.contains("▶ [83/92] Roadmap checks\n  done in 1.5 s\n"),
        "last step announced: {text}"
    );
    assert!(
        text.ends_with("  done in 1.5 s\n✔ all 92 checks passed in 2 min 18 s\n"),
        "summary: {text}"
    );
}

#[test]
fn stops_at_the_first_failing_command() {
    let mut fake = passing();
    fake.fail = Some("cargo test --workspace --locked -- --include-ignored".to_owned());
    let mut out = Vec::new();
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut fake, &mut out),
        failed(
            "Rust tests: `cargo test --workspace --locked -- --include-ignored` exited with code 3"
        ),
        "failure is named"
    );
    assert_eq!(
        fake.ran.last().map(String::as_str),
        Some("cargo test --workspace --locked -- --include-ignored"),
        "nothing runs after the failure"
    );
    assert!(
        !fake.ran.iter().any(|line| line.starts_with("cargo mutants")
            || line.starts_with("cargo +nightly-2026-11-01 llvm-cov")),
        "the longer steps never start"
    );
}

#[test]
fn reports_commands_that_cannot_start() {
    let mut fake = passing();
    fake.broken = Some("cargo fmt --all --check".to_owned());
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut fake, &mut Vec::new()),
        failed("Rust formatting: cannot run `cargo fmt --all --check`: not found"),
        "start failure is named"
    );
}

#[test]
fn shows_command_lines() {
    let cmd = Cmd {
        program: "cargo",
        args: vec!["fmt".to_owned(), "--check".to_owned()],
        dir: Dir::Root,
    };
    assert_eq!(cmd.line(), "cargo fmt --check", "program and arguments");
    let bare = Cmd {
        program: "pnpm",
        args: vec![],
        dir: Dir::Root,
    };
    assert_eq!(bare.line(), "pnpm", "no trailing space");
}

#[test]
fn reports_a_directory_that_cannot_be_emptied() {
    let mut fake = passing();
    fake.broken = Some("fresh target/conformance".to_owned());
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut fake, &mut Vec::new()),
        failed("Fresh conformance export directory: cannot empty target/conformance: busy"),
        "named and explained"
    );
}

#[test]
fn fails_when_unviable_files_are_missing_or_unreviewed() {
    let mut no_report = passing();
    let removed_report = no_report.files.remove(UNVIABLE_REPORT);
    assert!(removed_report.is_some(), "report removed");
    let full = Mode::Full {
        jobs: NonZeroUsize::MIN,
    };
    assert_eq!(
        run_steps(&check_steps(full), &mut no_report, &mut Vec::new()),
        failed(format!(
            "Rust unviable mutants reviewed: cannot read {UNVIABLE_REPORT}: missing"
        )),
        "a full run always writes its report"
    );
    let mut no_reviews = passing();
    let removed_reviews = no_reviews.files.remove(UNVIABLE_REVIEWED);
    assert!(removed_reviews.is_some(), "reviews removed");
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut no_reviews, &mut Vec::new()),
        failed(format!(
            "Every function a review of an unviable mutant names exists: cannot read \
             {UNVIABLE_REVIEWED}: missing"
        )),
        "missing reviews"
    );
    let mut unreviewed = passing();
    let _previous = unreviewed
        .files
        .insert(UNVIABLE_REVIEWED.to_owned(), String::new());
    assert!(
        run_steps(&check_steps(CHANGES_MODE), &mut unreviewed, &mut Vec::new())
            .is_err_and(|error| error.summary.contains("not reviewed: crates/a/src/b.rs")),
        "unreviewed mutant"
    );
}

#[test]
fn fails_when_equivalent_files_cannot_be_read() {
    let mut no_list = passing();
    no_list.broken = Some("list *.ts *.tsx".to_owned());
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut no_list, &mut Vec::new()),
        failed(
            "TypeScript equivalent mutants reviewed: cannot list the TypeScript files: `git ls-files -- *.ts *.tsx` exited with code 128: fatal: not a git repository"
        ),
        "no listing"
    );
    let mut no_reviews = passing();
    let removed = no_reviews.files.remove(EQUIVALENT_REVIEWED);
    assert!(removed.is_some(), "reviews removed");
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut no_reviews, &mut Vec::new()),
        failed(format!(
            "TypeScript equivalent mutants reviewed: cannot read {EQUIVALENT_REVIEWED}: missing"
        )),
        "missing reviews"
    );
}

#[test]
fn release_gate_runs_on_the_real_files() {
    let mut fake = passing();
    let replaced = fake.files.insert(CHANGELOG.to_owned(), "nope".to_owned());
    assert!(replaced.is_some(), "changelog replaced");
    assert!(
        run_steps(&check_steps(CHANGES_MODE), &mut fake, &mut Vec::new()).is_err_and(|error| {
            error.summary.starts_with(
                "One version everywhere, changelog well formed, released sections as on main: ",
            )
        }),
        "release gate failure is named"
    );
    for path in [CARGO_TOML.to_owned(), CHANGELOG.to_owned()]
        .into_iter()
        .chain(package_jsons())
    {
        let mut without = passing();
        let removed = without.files.remove(&path);
        assert!(removed.is_some(), "{path} removed");
        // The release step alone: the membership gate before it also notices a removal.
        let release: Vec<Step> = check_steps(CHANGES_MODE)
            .into_iter()
            .filter(|step| step.action == Action::VersionGate)
            .collect();
        assert_eq!(
            run_steps(&release, &mut without, &mut Vec::new()),
            failed(format!(
                "One version everywhere, changelog well formed, released sections as on main: \
                 cannot read {path}: missing"
            )),
            "missing {path}"
        );
    }
}

#[test]
fn fails_on_a_missing_or_failing_typescript_report() {
    let mut no_summary = passing();
    let removed = no_summary
        .files
        .remove("apps/playground/coverage/coverage-summary.json");
    assert!(removed.is_some(), "summary removed");
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut no_summary, &mut Vec::new()),
        failed(
            "TypeScript coverage and mutation reports of apps/playground: cannot read apps/playground/coverage/coverage-summary.json: missing"
        ),
        "missing summary"
    );
    let mut survivor = passing();
    let replaced = survivor.files.insert(
        "apps/roadmap/reports/mutation/mutation.json".to_owned(),
        one_mutant("Survived", "M", 4),
    );
    assert!(replaced.is_some(), "report replaced");
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut survivor, &mut Vec::new()),
        failed(
            "TypeScript coverage and mutation reports of apps/roadmap: apps/roadmap: mutants not killed: \
             apps/roadmap/x.ts:4 M Survived"
        ),
        "survivor"
    );
    let mut busy = passing();
    busy.broken = Some("fresh apps/playground/coverage".to_owned());
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut busy, &mut Vec::new()),
        failed("Fresh TypeScript report directories: cannot empty apps/playground/coverage: busy"),
        "a report directory that cannot be emptied"
    );
}

#[test]
fn reports_manifests_it_cannot_list() {
    let mut broken = passing();
    broken.broken = Some("list *Cargo.toml".to_owned());
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut broken, &mut Vec::new()),
        failed(
            "Packages and crates known to the checks: cannot list the *Cargo.toml files: `git ls-files -- *Cargo.toml` exited with code 128: fatal: not a git repository"
        ),
        "named"
    );
    let mut unlisted = passing();
    unlisted.broken = Some("list *package.json".to_owned());
    assert!(
        run_steps(&check_steps(CHANGES_MODE), &mut unlisted, &mut Vec::new())
            .is_err_and(|error| error.summary.ends_with("cannot list the *package.json files: `git ls-files -- *package.json` exited with code 128: fatal: not a git repository")),
        "named"
    );
}

#[test]
fn reports_license_lists_it_cannot_get() {
    let mut fake = passing();
    let removed = fake.files.remove(&format!("capture {}", LICENSE_LINES[0]));
    assert!(removed.is_some(), "report removed");
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut fake, &mut Vec::new()),
        failed(format!(
            "JavaScript licenses: cannot list licenses: `{}` exited with code 1: failed",
            LICENSE_LINES[0]
        )),
        "named"
    );
}

#[test]
fn stops_at_a_missing_tool() {
    let mut fake = passing();
    fake.broken = Some("cargo mutants --version".to_owned());
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut fake, &mut Vec::new()),
        failed(
            "Tools and platform: the checks cannot run here: cannot run `cargo mutants --version`: not found; to install: cargo install --locked cargo-mutants@27.1.0"
        ),
        "named, with how to install it"
    );
}

#[test]
fn names_every_check_alike() {
    for step in check_steps(CHANGES_MODE) {
        assert!(
            step.name
                .starts_with(|first: char| first.is_ascii_uppercase()),
            "{} starts with a capital letter",
            step.name
        );
        assert!(
            !step.name.ends_with('.'),
            "{} has no final period",
            step.name
        );
    }
}

/// Whether the verdict of a gate doing `action` carries its problems alone, the step's name
/// being their one source (`crate::steps::step_verdict`): every action is classed here, with no
/// wildcard, so that a new one does not compile before it is. `false` for a step that is
/// no gate, or for a gate whose verdict opens with a context of its own: the file or the
/// register it reads, or what it found, never a copy of its step's name.
const fn carries_its_problems_alone(action: &Action) -> bool {
    match action {
        Action::TestCodeGate
        | Action::TestDataGate
        | Action::UntestedTargetsGate
        | Action::UnviableNamesGate
        | Action::PinGate
        | Action::LibraryGate
        | Action::PartLicenseGate
        | Action::CitationGate
        | Action::LoadedFilesGate => true,
        Action::BypassGate
        | Action::CargoConfigGate
        | Action::DependencyGate
        | Action::EquivalentGate
        | Action::FloatMethodGate
        | Action::HistoryGate
        | Action::LicenseGate
        | Action::MathLayerGate
        | Action::MembershipGate
        | Action::PartsGate
        | Action::ReadmeGate
        | Action::StrykerConfigGate
        | Action::SupplyChainGate
        | Action::TypeScriptMutationGate { .. }
        | Action::TypeScriptReportsGate(_)
        | Action::TypeScriptScopeGate(_)
        | Action::UnsafeGate
        | Action::UnviableGate(_)
        | Action::UnviableReviewsCurrent
        | Action::ShardsUnviableGate(_)
        | Action::VersionGate
        | Action::VocabularyGate
        | Action::WorkflowGate
        | Action::Run(_)
        | Action::Coverage(_)
        | Action::CoverageLocality
        | Action::RustMutantsOfChanges { .. }
        | Action::RustMutantsOfChangedLines { .. }
        | Action::Bless(_)
        | Action::Conformance
        | Action::MutationProofRecorded
        | Action::FreshDirs(_)
        | Action::FusedBuild
        | Action::TreeRecorded
        | Action::TreeUnchanged
        | Action::Preflight => false,
    }
}

/// How many times `text` names `step`, or one of its clauses (`; `), its first letter in
/// either case.
fn times_named(step: &str, text: &str) -> usize {
    let lowered = text.to_lowercase();
    step.split("; ")
        .map(|clause| lowered.matches(&clause.to_lowercase()).count())
        .sum()
}

#[test]
fn names_each_failed_step_once() {
    let refused = |result: Result<(), Failure>| result.expect_err("refused").summary;
    let rust = |text: &str| vec![("crates/audio-core/src/a.rs".to_owned(), text.to_owned())];
    let (real, _) = fixture_metadata();
    let shared = r#"{"workspace_root":"/r","packages":[{"name":"x","dependencies":[{"name":"audio-core","kind":"dev","path":"/r/crates/audio-core"}]}]}"#;
    let alone = [
        (
            Action::LoadedFilesGate,
            vec![refused(crate::loaded_files_gate(
                &[crate::Loaded {
                    krate: crate::RUST_CRATES[0],
                    target: "audio_core".to_owned(),
                    configuration: None,
                    of_tests: false,
                    files: vec!["spec/x.json".to_owned()],
                }],
                &[],
            ))],
        ),
        (
            Action::TestCodeGate,
            vec![refused(test_code_gate(&rust(
                "#[cfg(test)]\nmod tests {}\n",
            )))],
        ),
        (
            Action::TestDataGate,
            vec![refused(test_data_gate(&rust(
                "const A: &str = include_str!(\"../../../spec/x.json\");\n",
            )))],
        ),
        (
            Action::UntestedTargetsGate,
            vec![refused(untested_targets_gate(
                &targets_metadata(&[("lib", "crates/x/src/lib.rs", true, false)]),
                &[],
            ))],
        ),
        (
            Action::UnviableNamesGate,
            vec![refused(unviable_names_gate(
                "a: b -- killed by no_such_test\n",
                &[],
            ))],
        ),
        (Action::PinGate, vec![refused(crate::pin_gate(&real, ""))]),
        (
            Action::LibraryGate,
            vec![
                refused(crate::library_gate(&real)),
                refused(crate::shared_tests_gate(shared)),
            ],
        ),
        (
            Action::PartLicenseGate,
            vec![refused(crate::part_license_gate(METADATA, &[], &[]))],
        ),
        (Action::CitationGate, vec![refused(cite("\u{23}12 step 4"))]),
    ];
    let steps = check_steps(Mode::Full { jobs: FAKE_JOBS });
    let classed: Vec<&Action> = steps
        .iter()
        .map(|step| &step.action)
        .filter(|action| carries_its_problems_alone(action))
        .collect();
    assert_eq!(
        classed,
        alone.iter().map(|(action, _)| action).collect::<Vec<_>>(),
        "every gate whose verdict carries its problems alone is refused below, in the order of \
         the steps"
    );
    for (action, summaries) in alone {
        let step = steps
            .iter()
            .find(|step| step.action == action)
            .unwrap_or_else(|| panic!("{action:?} is a step"));
        for summary in summaries {
            assert_eq!(
                times_named(&step.name, &summary),
                0,
                "{}: {summary}",
                step.name
            );
        }
    }
}

#[test]
fn names_a_failed_step_once_as_the_user_reads_it() {
    let steps = check_steps(Mode::Full { jobs: FAKE_JOBS });
    let step = steps
        .iter()
        .find(|step| step.action == Action::TestCodeGate)
        .expect("the test code gate is a step")
        .clone();
    let mut fake = passing();
    drop(fake.files.insert(
        "crates/audio-core/src/a.rs".to_owned(),
        "#[cfg(test)]\nmod tests {}\n".to_owned(),
    ));
    let summary = run_steps(core::slice::from_ref(&step), &mut fake, &mut Vec::new())
        .expect_err("refused")
        .summary;
    assert_eq!(times_named(&step.name, &summary), 1, "{summary}");
    assert!(
        summary.starts_with(&format!("{}: crates/audio-core/src/a.rs:1: ", step.name)),
        "{summary}"
    );
    assert_eq!(
        times_named("A b; c d", "x a B, C d"),
        2,
        "each clause, in either case"
    );
}

#[test]
fn checks_the_pins_in_the_history_of_main() {
    let mut unpinned = passing();
    let listed = unpinned.files.insert(
        format!("capture {}", PINS_LISTED.0),
        "fed4560000000000000000000000000000000000\n\
             a6614dfa18a3b69634f7f581f711d3f509bb5115\n\
             431b8a141a99759fb5c2a5adefd4fc551c43276c\n"
            .to_owned(),
    );
    assert!(listed.is_some(), "the passing fake lists the pins");
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut unpinned, &mut Vec::new()),
        failed(
            "Commit messages, and a changelog entry for each change of behavior: the history \
             gate is pinned to commits main does not hold: SCOPES_SINCE names \
             9853e43382d0af0e44970c8304294d34e0d8ac2b, which is not in the history of \
             fed4560000000000000000000000000000000000 (merge base with origin/main): pin a \
             commit main holds"
        ),
        "a branch-only pin, refused before the merge rewrites it"
    );
    let mut unread = passing();
    let _removed = unread.files.remove(&format!("capture {}", PINS_LISTED.0));
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut unread, &mut Vec::new()),
        failed(
            "Commit messages, and a changelog entry for each change of behavior: cannot read \
             the history of origin/main: `git log --format=%H \
             fed4560000000000000000000000000000000000` exited with code 1: failed"
        ),
        "a history git cannot list"
    );
}

#[test]
fn reads_the_history_or_says_why_it_cannot() {
    let mut fake = passing();
    let _removed = fake.files.remove(&format!("capture {}", HISTORY_LINES[0]));
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut fake, &mut Vec::new()),
        failed(
            "Commit messages, and a changelog entry for each change of behavior: cannot read the history: `git log --format=%x1e%H%x1f%B HEAD` exited with code 1: failed"
        ),
        "a shallow clone, or no git"
    );
}

#[test]
fn shows_one_problem_inline_and_lists_several() {
    assert_eq!(
        crate::steps::verdict("context", vec![]),
        Ok(()),
        "no problem"
    );
    assert_eq!(
        crate::steps::verdict("context", vec!["a".to_owned()]),
        Err(Failure {
            summary: "context: a".to_owned(),
            details: vec![],
            usage: false,
        }),
        "one problem, in the summary line"
    );
    assert_eq!(
        crate::steps::verdict("context", vec!["a".to_owned(), "b".to_owned()]),
        Err(Failure {
            summary: "context: 2 problems, listed above".to_owned(),
            details: vec!["a".to_owned(), "b".to_owned()],
            usage: false,
        }),
        "several problems, counted, then listed"
    );
}

#[test]
fn wires_every_package_into_the_checks() {
    let steps = check_steps(CHANGES_MODE);
    let fresh: Vec<String> = steps
        .iter()
        .filter_map(|step| {
            if let Action::FreshDirs(paths) = &step.action {
                Some(paths.clone())
            } else {
                None
            }
        })
        .flatten()
        .collect();
    let [shipped, _] = crate::licenses::license_commands();
    for package in PACKAGES {
        let checked = steps.iter().position(|step| {
            step.name == package.step
                && step.action
                    == Action::Run(Cmd {
                        program: "pnpm",
                        args: vec!["run".to_owned(), "check".to_owned()],
                        dir: Dir::Package(package.dir),
                    })
        });
        let gated = steps
            .iter()
            .position(|step| step.action == Action::TypeScriptReportsGate(package));
        assert!(
            checked.is_some() && checked < gated,
            "{}: its checks run, before the reports gate",
            package.dir
        );
        for report in package.reports() {
            assert!(fresh.contains(&report), "{report} is emptied first");
        }
        assert!(
            package_jsons().contains(&format!("{}/package.json", package.dir)),
            "{}: its manifest is versioned",
            package.dir
        );
        assert_eq!(
            shipped.args.contains(&format!("./{}", package.dir)),
            package.shipped,
            "{}: its production licenses are held to the shipped list if it ships",
            package.dir
        );
    }
}

#[test]
fn reports_readme_problems_whatever_the_membership() {
    let mut fake = passing();
    let extra = fake
        .files
        .insert("apps/editor/package.json".to_owned(), PACKAGE.to_owned());
    let removed = fake.files.remove("apps/README.md");
    assert!(extra.is_none() && removed.is_some(), "both problems set");
    let step = |action: &Action| -> Vec<Step> {
        check_steps(CHANGES_MODE)
            .into_iter()
            .filter(|step| step.action == *action)
            .collect()
    };
    assert_eq!(
        run_steps(&step(&Action::MembershipGate), &mut fake, &mut Vec::new()),
        failed(
            "Packages and crates known to the checks: the manifests differ from what xtask \
             knows: apps/editor/package.json is not a package of PACKAGES: its checks and \
             reports would not run"
        ),
        "the membership problem, under its own step"
    );
    assert_eq!(
        run_steps(&step(&Action::ReadmeGate), &mut fake, &mut Vec::new()),
        failed("Every part described in a README: parts not described: apps has no README.md"),
        "the README problems, not hidden by the membership one"
    );
}

/// What a cargo command builds: the subcommand, the target (none for the host's), the
/// `--config` and `--target-dir` given, and the packages (`--workspace` as `*`).
type Built = (String, [Option<String>; 3], Vec<String>);

/// What a cargo command builds ([`Built`]), if it builds.
fn built_configuration(cmd: &Cmd) -> Option<Built> {
    let subcommand = cmd.args.first()?;
    if cmd.program != "cargo"
        || !["build", "check", "clippy", "test"].contains(&subcommand.as_str())
    {
        return None;
    }
    let value = |flag: &str| {
        cmd.args
            .iter()
            .skip_while(|arg| *arg != flag)
            .nth(1)
            .cloned()
    };
    let mut packages: Vec<String> = cmd
        .args
        .windows(2)
        .filter(|pair| pair[0] == "--package")
        .map(|pair| pair[1].clone())
        .collect();
    if cmd.args.iter().any(|arg| arg == "--workspace") {
        packages.push("*".to_owned());
    }
    Some((
        subcommand.clone(),
        [value("--target"), value("--config"), value("--target-dir")],
        packages,
    ))
}

#[test]
fn reads_every_configuration_the_check_builds() {
    let gate: Vec<_> = crate::loaded_files_commands()
        .iter()
        .map(|command| built_configuration(&command.cmd).expect("a build"))
        .collect();
    let steps: Vec<_> = check_steps(Mode::Full { jobs: FAKE_JOBS })
        .into_iter()
        .filter_map(|step| {
            if let Action::Run(cmd) = &step.action {
                built_configuration(cmd)
            } else if step.action == Action::FusedBuild {
                built_configuration(&crate::fused_command())
            } else {
                None
            }
        })
        .collect();
    let configurations = |builds: &[Built]| {
        let mut found: Vec<[Option<String>; 3]> = builds
            .iter()
            .map(|(_, configuration, _)| configuration.clone())
            .collect();
        found.sort();
        found.dedup();
        found
    };
    assert_eq!(
        configurations(&gate),
        configurations(&steps),
        "the gate reads every configuration the check builds, and no other"
    );
    for (subcommand, configuration, packages) in &steps {
        assert!(
            gate.iter()
                .any(|(_, read, read_packages)| read == configuration
                    && (read_packages.contains(&"*".to_owned())
                        || packages
                            .iter()
                            .all(|package| read_packages.contains(package)))),
            "every package of `cargo {subcommand}` in {configuration:?} read: {packages:?}"
        );
    }
    for (_, [target, _, _], packages) in &gate {
        assert_eq!(
            packages.contains(&"audio-wasm".to_owned()),
            target.as_deref() == Some("wasm32-unknown-unknown"),
            "the engine's crate for WebAssembly, checked for WebAssembly alone: {target:?}"
        );
    }
    let fused = crate::fused_command();
    let before_harness: Vec<&str> = fused
        .args
        .iter()
        .map(String::as_str)
        .take_while(|&arg| arg != "--")
        .collect();
    assert_eq!(
        crate::loaded_files_commands()[2].cmd.args,
        [
            before_harness,
            vec!["--no-run", "--message-format=json-render-diagnostics"]
        ]
        .concat(),
        "the fused build's own units, not run"
    );
}

#[test]
fn runs_miri_on_every_crate_of_the_audited_files() {
    assert_eq!(
        crate::unsafe_crates(),
        vec!["alloc-counter", "audio-wasm"],
        "the crates of the audited files, in the order of RUST_CRATES, but the assembly one"
    );
    assert_eq!(
        crate::ASSEMBLY_FILES,
        ["crates/audio-fpenv/src/word.rs"],
        "the one file Miri cannot run"
    );
    assert!(
        crate::ASSEMBLY_FILES
            .iter()
            .all(|file| UNSAFE_FILES.contains(file)),
        "an audited file"
    );
    assert_eq!(
        crate::miri_command().line(),
        format!(
            "cargo +nightly-2026-11-01 {} test --package alloc-counter --package \
             audio-wasm --locked -- --include-ignored",
            "miri"
        ),
        "Miri on each of them, ignored tests included"
    );
    assert!(
        check_steps(CHANGES_MODE)
            .iter()
            .any(|step| step.action == Action::Run(crate::miri_command())),
        "a step of the check"
    );
}

#[test]
fn empties_the_coverage_build_before_every_measurement() {
    let mut fake = passing();
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut fake, &mut Vec::new()),
        Ok(()),
        "all green"
    );
    let clean = Coverage::clean_command().line();
    let measurements: Vec<usize> = fake
        .ran
        .iter()
        .enumerate()
        .filter(|(_, line)| line.contains("llvm-cov --manifest-path"))
        .map(|(index, _)| index)
        .collect();
    assert_eq!(
        measurements.len(),
        13 + 1,
        "every crate and binary measured, and the locality fixture"
    );
    for index in measurements {
        assert_eq!(
            fake.ran.get(index.wrapping_sub(1)),
            Some(&clean),
            "a measurement right after a clean coverage build"
        );
    }
}

#[test]
fn stops_when_the_coverage_build_cannot_be_emptied() {
    let mut fake = passing();
    fake.broken = Some(Coverage::clean_command().line());
    let failure =
        run_steps(&check_steps(CHANGES_MODE), &mut fake, &mut Vec::new()).expect_err("stops");
    assert!(
        failure.summary.contains("not found"),
        "the reason the clean failed: {}",
        failure.summary
    );
    assert!(
        fake.ran
            .iter()
            .all(|line| !line.contains("llvm-cov --manifest-path")),
        "nothing is measured on a build that could not be emptied"
    );
}

#[test]
fn fails_when_a_supply_chain_file_cannot_be_read() {
    for path in crate::SUPPLY_CHAIN_FILES {
        let mut fake = passing();
        let _removed = fake.files.remove(path);
        let failure =
            run_steps(&check_steps(CHANGES_MODE), &mut fake, &mut Vec::new()).expect_err("stops");
        assert!(
            failure.summary.contains(path),
            "names the file: {}",
            failure.summary
        );
    }
    let mut fake = passing();
    let _npmrc = fake.files.insert("apps/x/.npmrc".to_owned(), String::new());
    let failure =
        run_steps(&check_steps(CHANGES_MODE), &mut fake, &mut Vec::new()).expect_err("stops");
    assert!(
        failure
            .summary
            .contains("apps/x/.npmrc: an npm configuration file"),
        "the tracked npm configuration files: {}",
        failure.summary
    );
}

#[test]
fn requires_the_fused_build_to_have_run_with_its_option() {
    assert_eq!(
        crate::fused_gate(&format!("a\n{}\nb", crate::FUSED_PROOF)),
        Ok(()),
        "the proof line"
    );
    for output in [
        String::new(),
        "test result: ok. 81 passed".to_owned(),
        format!("{} extra", crate::FUSED_PROOF),
        crate::FUSED_PROOF.replace("... ok", "... FAILED"),
    ] {
        assert_eq!(
            crate::fused_gate(&output),
            failed(format!(
                "the fused test build ran without its option: `{}` is missing from its output",
                crate::FUSED_PROOF
            )),
            "{output:?}"
        );
    }
    let command = crate::fused_command().line();
    assert!(
        command.contains(r#"--config build.rustflags=["--cfg", "poyetik_math=\"fused\""]"#)
            && command.contains("--target-dir target/fused")
            && command.contains("--package audio-core --package audio-boxes"),
        "{command}"
    );
    let steps: Vec<Step> = check_steps(CHANGES_MODE)
        .into_iter()
        .filter(|step| step.action == Action::FusedBuild)
        .collect();
    assert_eq!(steps.len(), 1, "one fused build");
    let mut broken = passing();
    let _removed = broken
        .files
        .remove(&format!("capture {}", crate::fused_command().line()));
    let failure = run_steps(&steps, &mut broken, &mut Vec::new()).expect_err("stops");
    assert!(
        failure
            .summary
            .contains(": the fused test build failed: `cargo test --locked"),
        "{}",
        failure.summary
    );
    let mut silent = passing();
    let _replaced = silent.files.insert(
        format!("capture {}", crate::fused_command().line()),
        "test result: ok.\n".to_owned(),
    );
    assert!(
        run_steps(&steps, &mut silent, &mut Vec::new()).is_err(),
        "a build without the option fails"
    );
}

#[test]
fn the_math_layer_gate_fails_when_the_crates_cannot_be_listed() {
    let steps: Vec<Step> = check_steps(CHANGES_MODE)
        .into_iter()
        .filter(|step| step.action == Action::MathLayerGate)
        .collect();
    assert_eq!(steps.len(), 1, "one math layer gate");
    let mut fake = passing();
    let _removed = fake
        .files
        .remove(&format!("capture {}", crate::metadata_command().line()));
    let failure = run_steps(&steps, &mut fake, &mut Vec::new()).expect_err("stops");
    assert!(
        failure.summary.contains("cannot list the crates"),
        "says what it could not do: {}",
        failure.summary
    );
    assert_eq!(
        run_steps(&steps, &mut passing(), &mut Vec::new()),
        Ok(()),
        "the fake's metadata has no libm"
    );
}

#[test]
fn makes_a_message_safe_to_show() {
    use crate::{MAX_MESSAGE_CHARS, safe_message};
    assert_eq!(
        safe_message("commit abc: unknown type \"x\""),
        "commit abc: unknown type \"x\"",
        "plain text is unchanged, quotes included"
    );
    let hostile = "a\nb\rc\td\u{7f}e\u{85}f\u{202e}g\u{2066}h\u{200f}i\u{200e}i\u{61c}j\u{2028}k\u{2029}l\u{202a}m\u{2069}n\u{1b}";
    assert_eq!(
        safe_message(hostile),
        "a\\u{a}b\\u{d}c\\u{9}d\\u{7f}e\\u{85}f\\u{202e}g\\u{2066}h\\u{200f}i\\u{200e}i\\u{61c}j\\u{2028}k\\u{2029}l\\u{202a}m\\u{2069}n\\u{1b}",
        "control, bidi and line-separator characters are escaped"
    );
    assert_eq!(
        safe_message("é ü 音 \u{202f}\u{206a}\u{2065}"),
        "é ü 音 \u{202f}\u{206a}\u{2065}",
        "other text is kept, the neighbours of the escaped ranges included"
    );
    assert_eq!(safe_message(""), "", "nothing to show");
    let at_limit = "x".repeat(MAX_MESSAGE_CHARS);
    assert_eq!(safe_message(&at_limit), at_limit, "exactly at the limit");
    assert_eq!(
        safe_message(&format!("{at_limit}y")),
        format!("{}…", "x".repeat(MAX_MESSAGE_CHARS - 1)),
        "one character more: cut, and marked"
    );
    assert_eq!(
        safe_message(&"\n".repeat(MAX_MESSAGE_CHARS)),
        format!("{}…", "\\u{a}".repeat((MAX_MESSAGE_CHARS - 1) / 5)),
        "whole escapes only"
    );
    assert_eq!(
        MAX_MESSAGE_CHARS, 1_000,
        "the limit of audio_patch::safe_message"
    );
}

/// Whether `text` is lines of at most `max` characters, none holding a control character.
fn safe_lines(text: &[u8], max: usize) -> bool {
    String::from_utf8(text.to_vec()).is_ok_and(|text| {
        text.lines()
            .all(|line| line.chars().count() <= max && !line.chars().any(char::is_control))
    })
}

#[test]
fn shows_untrusted_text_on_one_bounded_line() {
    let line_max = "error: ".len() + crate::MAX_MESSAGE_CHARS;
    let args = ["commit-msg".to_owned(), "COMMIT_EDITMSG".to_owned()];
    for (message, shown) in [
        (
            "\u{1b}]0;owned\u{7} feat: x\n".to_owned(),
            "error: invalid commit message: unknown type \"\\u{1b}]0;owned\\u{7} feat\"\n"
                .to_owned(),
        ),
        (
            format!("{}: y\n", "x".repeat(200_000)),
            format!(
                "error: {}…\n",
                format!(
                    "invalid commit message: unknown type \"{}",
                    "x".repeat(1_000)
                )
                .chars()
                .take(crate::MAX_MESSAGE_CHARS - 1)
                .collect::<String>()
            ),
        ),
    ] {
        let mut fake = passing();
        let _replaced = fake.files.insert("COMMIT_EDITMSG".to_owned(), message);
        let (mut out, mut err) = (Vec::new(), Vec::new());
        assert_eq!(run_cli(&args, &mut fake, &mut out, &mut err), 1, "refused");
        assert_eq!(String::from_utf8(err).expect("UTF-8"), shown, "one line");
        assert_eq!(
            String::from_utf8(out).expect("UTF-8"),
            "expected: <type>(<scope>)!: <summary>, type one of feat, fix, docs, test, refactor, \
             perf, build, ci, chore, style, revert\n",
            "the details"
        );
    }
    let (mut out, mut err) = (Vec::new(), Vec::new());
    assert_eq!(
        run_cli(
            &["\u{1b}[2J\nerror: forged".to_owned()],
            &mut passing(),
            &mut out,
            &mut err
        ),
        2,
        "a usage error"
    );
    assert_eq!(
        String::from_utf8(err).expect("UTF-8"),
        "error: unknown command \"\\u{1b}[2J\\u{a}error: forged\" (see: cargo xtask help)\n",
        "one line, the argument escaped"
    );
    assert!(out.is_empty(), "nothing on stdout");

    let mut hostile = passing();
    let _replaced = hostile.files.insert(
        crate::CARGO_CONFIG_FILE.to_owned(),
        format!(
            "[build]\u{1b}[8m\nrustflags = [\"{}\"]\n",
            "x".repeat(5_000)
        ),
    );
    let (mut check_out, mut check_err) = (Vec::new(), Vec::new());
    assert_eq!(
        run_cli(
            &["check".to_owned()],
            &mut hostile,
            &mut check_out,
            &mut check_err
        ),
        1,
        "a configuration refused"
    );
    let details = String::from_utf8(check_out.clone()).expect("UTF-8");
    assert!(
        details.contains("\nfound:\n[build]\\u{1b}[8m\nrustflags = [\"xxx"),
        "each line of a detail on its own line, escaped: {details}"
    );
    assert!(
        safe_lines(&check_out, crate::MAX_MESSAGE_CHARS),
        "bounded: {details}"
    );
    assert!(safe_lines(&check_err, line_max), "one bounded error line");
}
