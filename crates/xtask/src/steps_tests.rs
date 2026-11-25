//! Tests of the steps of the check: commands, actions, their verdicts, and the order they run in.

use core::num::NonZeroUsize;
use std::path::Path;

use crate::tests::{
    CHANGES_MODE, FAKE_JOBS, HISTORY_LINES, LICENSE_LINES, METADATA, PACKAGE, PINS_LISTED,
    added_builds, added_package_steps, cite, command_lines, elapsed, failed, fixture_metadata,
    one_mutant, passing, run_cli, scratch, targets_metadata, test_code_gate, test_data_gate,
    untested_targets_gate, unviable_names_gate, visual_binaries, visual_crates, visual_steps,
};
use crate::{
    Action, CARGO_TOML, CHANGELOG, Cmd, Coverage, Dir, EQUIVALENT_REVIEWED, Extent, Failure,
    MUTANTS_OUT, Mode, PACKAGES, Step, UNSAFE_FILES, UNVIABLE_REPORT, UNVIABLE_REVIEWED,
    check_steps, package_jsons, run_steps,
};

/// What each step of the default check does, in order, as `command_lines` says it.
const DEFAULT_PIPELINE: &[&str] = &[
    "preflight",
    "mutant left gate",
    "tree recorded",
    "Root: pnpm install --frozen-lockfile",
    "fresh tooling/config/coverage tooling/config/reports/mutation tooling/oracle/coverage tooling/oracle/reports/mutation tooling/wasm-build/coverage tooling/wasm-build/reports/mutation bindings/audio-wasm/coverage bindings/audio-wasm/reports/mutation bindings/visual-wasm/coverage bindings/visual-wasm/reports/mutation bindings/web-page/coverage bindings/web-page/reports/mutation apps/playground/coverage apps/playground/reports/mutation apps/roadmap/coverage apps/roadmap/reports/mutation",
    "fresh bindings/audio-wasm/dist bindings/visual-wasm/dist bindings/web-page/dist",
    "equivalent gate",
    "bypass gate",
    "unsafe gate",
    "library lints gate",
    "cargo config gate",
    "test code gate",
    "test data gate",
    "untested targets gate",
    "unviable names gate",
    "conformance cargo run --package audio-cli --locked --quiet -- check conformance --spent target/xtask/spent-algorithms.json",
    "Root: cargo run --release --package visual-cli --locked --quiet -- check conformance/visual",
    "Root: cargo deny --locked check",
    "Root: cargo deny --manifest-path crates/visual-gpu/Cargo.toml --locked check",
    "dependency gate",
    "pin gate",
    "build-time code gate",
    "library gate",
    "part license gate",
    "math layer gate",
    "supply chain gate",
    "CPU reference gate",
    "membership gate",
    "readme gate",
    "citation gate",
    "list counts gate",
    "vocabulary gate",
    "conflict marker gate",
    "source kinds gate",
    "retired patterns gate",
    "runnable hints gate",
    "hermetic git gate",
    "register names gate",
    "version gate",
    "history gate",
    "workflow gate",
    "parts gate",
    "Root: pnpm audit --audit-level=low",
    "license gate",
    "stryker config gate",
    "package check gate",
    "Root: cargo fmt --all --check",
    "Root: cargo clippy --workspace --all-targets --locked",
    "float method gate",
    "Root: cargo doc --workspace --no-deps --document-private-items --locked",
    "Root: cargo test --workspace --locked -- --include-ignored",
    "Root: cargo fmt --manifest-path crates/visual-gpu/Cargo.toml --all --check",
    "Root: cargo clippy --manifest-path crates/visual-gpu/Cargo.toml --workspace --all-targets --locked",
    "Root: cargo doc --manifest-path crates/visual-gpu/Cargo.toml --workspace --no-deps --document-private-items --locked",
    "Root: cargo test --manifest-path crates/visual-gpu/Cargo.toml --workspace --locked -- --include-ignored",
    "Root: cargo run --manifest-path crates/visual-gpu/Cargo.toml --locked --quiet -- check",
    "doc tests",
    "loaded files gate",
    "fused build",
    "Root: cargo build --package audio-core --package audio-boxes --package audio-patch --package audio-hash --package audio-host --package visual-core --package visual-boxes --package visual-hash --package visual-host --package coordinator --locked --target aarch64-apple-darwin",
    "Root: cargo build --package audio-core --package audio-boxes --package audio-patch --package audio-hash --package audio-host --package visual-core --package visual-boxes --package visual-hash --package visual-host --package coordinator --locked --target x86_64-unknown-linux-gnu",
    "Root: cargo build --package audio-core --package audio-boxes --package audio-patch --package audio-hash --package audio-host --package visual-core --package visual-boxes --package visual-hash --package visual-host --package coordinator --locked --target wasm32-unknown-unknown",
    "Root: cargo build --package audio-core --package audio-boxes --package audio-patch --package audio-hash --package audio-host --package visual-core --package visual-boxes --package visual-hash --package visual-host --package coordinator --locked --target thumbv7em-none-eabihf",
    "Root: cargo clippy --package fpenv --package audio-cli --all-targets --locked --target aarch64-apple-darwin",
    "Root: cargo clippy --package fpenv --package audio-cli --all-targets --locked --target x86_64-unknown-linux-gnu",
    "Root: cargo build --lib --package fpenv --package audio-cli --locked --target aarch64-apple-darwin",
    "Root: cargo build --lib --package fpenv --package audio-cli --locked --target x86_64-unknown-linux-gnu",
    "fresh target/coverage",
    "coverage locality",
    "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/audio-core/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/audio-core.json -- --include-ignored",
    "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/audio-boxes/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/audio-boxes.json -- --include-ignored",
    "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/audio-patch/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/audio-patch.json -- --include-ignored",
    "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/audio-hash/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/audio-hash.json -- --include-ignored",
    "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/audio-host/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/audio-host.json -- --include-ignored",
    "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/visual-core/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/visual-core.json -- --include-ignored",
    "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/visual-boxes/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/visual-boxes.json -- --include-ignored",
    "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/visual-hash/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/visual-hash.json -- --include-ignored",
    "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/visual-testkit/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/visual-testkit.json -- --include-ignored",
    "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/visual-cli/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/visual-cli.json -- --include-ignored",
    "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/visual-host/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/visual-host.json -- --include-ignored",
    "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/visual-gpu/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/visual-gpu.json -- --include-ignored",
    "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path bindings/visual-wasm/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/visual-wasm.json -- --include-ignored",
    "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/alloc-counter/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/alloc-counter.json -- --include-ignored",
    "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/fpenv/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/fpenv.json -- --include-ignored",
    "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/audio-battery/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/audio-battery.json -- --include-ignored",
    "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/audio-testkit/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/audio-testkit.json -- --include-ignored",
    "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/audio-cli/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/audio-cli.json -- --include-ignored",
    "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path bindings/audio-wasm/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/audio-wasm.json -- --include-ignored",
    "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/coordinator/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/coordinator.json -- --include-ignored",
    "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path bindings/web-page/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/web-page.json -- --include-ignored",
    "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/xtask/Cargo.toml --lib --locked --branch --json --summary-only --output-path target/coverage/xtask.json -- --include-ignored",
    "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/visual-cli/Cargo.toml --locked --branch --json --summary-only --output-path target/coverage/visual-cli-main.json -- --include-ignored",
    "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/visual-gpu/Cargo.toml --locked --branch --json --summary-only --output-path target/coverage/visual-gpu-main.json -- --include-ignored",
    "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/audio-cli/Cargo.toml --locked --branch --json --summary-only --output-path target/coverage/audio-cli-main.json -- --include-ignored",
    "coverage cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/xtask/Cargo.toml --locked --branch --json --summary-only --output-path target/coverage/xtask-main.json -- --include-ignored",
    concat!(
        "Root: cargo +nightly-2026-11-01 mi",
        "ri test --package visual-wasm --package alloc-counter --package audio-wasm --package web-page --locked -- --include-ignored"
    ),
    "Package(\"tooling/config\"): pnpm run typecheck",
    "Package(\"tooling/config\"): pnpm run lint",
    "Package(\"tooling/config\"): pnpm run test",
    "Package(\"tooling/config\"): pnpm exec stryker run stryker.config.json --incremental --incrementalFile reports/stryker-incremental.json",
    "typescript reports gate of tooling/config",
    "typescript scope gate of tooling/config",
    "fresh target/conformance",
    "Root: cargo run --package audio-cli --locked --quiet -- export conformance target/conformance",
    "Package(\"tooling/oracle\"): pnpm run typecheck",
    "Package(\"tooling/oracle\"): pnpm run lint",
    "Package(\"tooling/oracle\"): pnpm run test",
    "Package(\"tooling/oracle\"): pnpm run oracle",
    "Package(\"tooling/oracle\"): pnpm exec stryker run stryker.config.json --incremental --incrementalFile reports/stryker-incremental.json",
    "typescript reports gate of tooling/oracle",
    "typescript scope gate of tooling/oracle",
    "Package(\"tooling/wasm-build\"): pnpm run typecheck",
    "Package(\"tooling/wasm-build\"): pnpm run lint",
    "Package(\"tooling/wasm-build\"): pnpm run test",
    "Package(\"tooling/wasm-build\"): pnpm exec stryker run stryker.config.json --incremental --incrementalFile reports/stryker-incremental.json",
    "typescript reports gate of tooling/wasm-build",
    "typescript scope gate of tooling/wasm-build",
    "Package(\"bindings/audio-wasm\"): pnpm run build",
    "Package(\"bindings/audio-wasm\"): pnpm run typecheck",
    "Package(\"bindings/audio-wasm\"): pnpm run lint",
    "Package(\"bindings/audio-wasm\"): pnpm run test",
    "Package(\"bindings/audio-wasm\"): pnpm run benchmarks:check",
    "Package(\"bindings/audio-wasm\"): pnpm run test:allocation",
    "Package(\"bindings/audio-wasm\"): pnpm run test:reproducible",
    "Package(\"bindings/audio-wasm\"): pnpm run test:e2e",
    "Package(\"bindings/audio-wasm\"): pnpm exec stryker run stryker.config.json --incremental --incrementalFile reports/stryker-incremental.json",
    "typescript reports gate of bindings/audio-wasm",
    "typescript scope gate of bindings/audio-wasm",
    "Package(\"bindings/visual-wasm\"): pnpm run build",
    "Package(\"bindings/visual-wasm\"): pnpm run typecheck",
    "Package(\"bindings/visual-wasm\"): pnpm run lint",
    "Package(\"bindings/visual-wasm\"): pnpm run test",
    "Package(\"bindings/visual-wasm\"): pnpm run test:reproducible",
    "Package(\"bindings/visual-wasm\"): pnpm run benchmarks:check",
    "Package(\"bindings/visual-wasm\"): pnpm exec stryker run stryker.config.json --incremental --incrementalFile reports/stryker-incremental.json",
    "typescript reports gate of bindings/visual-wasm",
    "typescript scope gate of bindings/visual-wasm",
    "Package(\"bindings/visual-wasm\"): pnpm run build",
    "Package(\"bindings/web-page\"): pnpm run build",
    "Package(\"bindings/web-page\"): pnpm run typecheck",
    "Package(\"bindings/web-page\"): pnpm run lint",
    "Package(\"bindings/web-page\"): pnpm run test",
    "Package(\"bindings/web-page\"): pnpm run test:reproducible",
    "Package(\"bindings/web-page\"): pnpm exec stryker run stryker.config.json --incremental --incrementalFile reports/stryker-incremental.json",
    "typescript reports gate of bindings/web-page",
    "typescript scope gate of bindings/web-page",
    "Package(\"bindings/audio-wasm\"): pnpm run build",
    "Package(\"bindings/web-page\"): pnpm run build",
    "Package(\"apps/playground\"): pnpm run typecheck",
    "Package(\"apps/playground\"): pnpm run lint",
    "Package(\"apps/playground\"): pnpm run test",
    "Package(\"apps/playground\"): pnpm run build",
    "Package(\"apps/playground\"): pnpm run test:e2e",
    "Package(\"apps/playground\"): pnpm exec stryker run stryker.config.json --incremental --incrementalFile reports/stryker-incremental.json",
    "typescript reports gate of apps/playground",
    "typescript scope gate of apps/playground",
    "Package(\"apps/roadmap\"): pnpm run typecheck",
    "Package(\"apps/roadmap\"): pnpm run lint",
    "Package(\"apps/roadmap\"): pnpm run test",
    "Package(\"apps/roadmap\"): pnpm run build",
    "Package(\"apps/roadmap\"): pnpm run test:e2e",
    "Package(\"apps/roadmap\"): pnpm exec stryker run stryker.config.json --incremental --incrementalFile reports/stryker-incremental.json",
    "typescript reports gate of apps/roadmap",
    "typescript scope gate of apps/roadmap",
    "fresh mutants.out",
    "rust mutants of the changes (3 jobs)",
    "unviable gate (Partial)",
    "fresh mutants.out-corpus",
    "corpus mutants (Changes { jobs: 3 })",
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
    "No mutant left in place in the GPU workspace",
    "Working tree recorded",
    "JavaScript dependencies",
    "Fresh TypeScript report directories",
    "Fresh TypeScript build outputs",
    "TypeScript equivalent mutants reviewed",
    "Every bypass of coverage or mutation testing reviewed",
    "Unsafe code confined to the audited files",
    "Every root of both libraries denies its lints; every `no_std` crate built for every portability target or listed",
    "Cargo configuration: no build flag, a locked alias, the GPU workspace sharing the root's settings",
    "Test code only in tests/ and declared *_tests.rs files; a Rust test compares an error whole",
    "The tests of each crate read only its declared test data; each Rust command-line tool's tests read the shared vectors",
    "Tests turned off only where no test can hide, doc tests kept, each compile_fail \
             naming its error code",
    "Every function a review or a specification names exists, each new review naming its test; each reader of a file only a tool writes is named in the index",
    "Conformance (native)",
    "Visual conformance (native)",
    "Rust supply chain",
    "Rust supply chain (GPU workspace)",
    "Every direct dependency reviewed",
    "Bit-defining dependencies pinned to one exact version",
    "Every third-party build script and procedural macro reviewed; the crates both lock files share at one version",
    "Each library depends only on its own crates, its tests also on its test crates; every crate only tests use is shared test code",
    "Every part under the license of its folder, no open part depending on an app",
    "Box arithmetic only through the math layer: libm in audio-core and audio-testkit only, no raw float in box code, no iterator reduction outside the math layer",
    "Supply-chain configuration as xtask defines it",
    "The CPU reference built in the GPU workspace as at the root",
    "Packages and crates known to the checks",
    "Every part described in a README",
    "Every roadmap step cited exists on the board",
    "No prose counts a typed list",
    "The words of the glossary, not their synonyms",
    "No conflict marker left in a tracked text file",
    "Every tracked text file of a known kind",
    "No pattern a fix retired, in the code it was retired from",
    "Every hint a message gives names a command a user can run",
    "Every place that starts a process is listed; every copy of git's variables is xtask's",
    "Every reason of the registers of proof bypasses and equivalent mutants names what exists",
    "One version everywhere, changelog well formed, released sections as on main",
    "Commit messages, and a changelog entry for each change of behavior",
    "CI runs exactly the workflow xtask defines",
    "The parts of CI run every check of the full check once",
    "JavaScript vulnerabilities",
    "JavaScript licenses",
    "Stryker configurations hide no mutant",
    "Each package's check runs exactly the scripts the checks time",
    "Rust formatting",
    "Rust lints",
    "Float methods of the pinned toolchain banned or reviewed as deterministic",
    "Rust documentation",
    "Rust tests",
    "Rust formatting (GPU workspace)",
    "Rust lints (GPU workspace)",
    "Rust documentation (GPU workspace)",
    "Rust tests (GPU workspace)",
    "Visual GPU time benchmark, checked only on a GPU the baseline holds (GPU workspace)",
    "Rust doc tests, the error code of each compile_fail checked",
    "Files rustc loads: tracked, in their crate, those of tests alone named as test code",
    "Boxes unchanged under the math layer's fused multiply-add option (a test build)",
    "Library portability (aarch64-apple-darwin)",
    "Library portability (x86_64-unknown-linux-gnu)",
    "Library portability (wasm32-unknown-unknown)",
    "Library portability (thumbv7em-none-eabihf)",
    "Native shell lints (aarch64-apple-darwin)",
    "Native shell lints (x86_64-unknown-linux-gnu)",
    "Native code of audio-cli and fpenv built, inline assembly included (aarch64-apple-darwin)",
    "Native code of audio-cli and fpenv built, inline assembly included (x86_64-unknown-linux-gnu)",
    "Fresh Rust coverage reports",
    "Rust coverage counts a crate's own unit tests only",
    "Rust coverage of audio-core: 100% by its own unit tests",
    "Rust coverage of audio-boxes: 100% by its own unit tests",
    "Rust coverage of audio-patch: 100% by its own unit tests",
    "Rust coverage of audio-hash: 100% by its own unit tests",
    "Rust coverage of audio-host: 100% by its own unit tests",
    "Rust coverage of visual-core: 100% by its own unit tests",
    "Rust coverage of visual-boxes: 100% by its own unit tests",
    "Rust coverage of visual-hash: 100% by its own unit tests",
    "Rust coverage of visual-testkit: 100% by its own unit tests",
    "Rust coverage of visual-cli: 100% by its own unit tests",
    "Rust coverage of visual-host: 100% by its own unit tests",
    "Rust coverage of visual-gpu: 100% by its own unit tests",
    "Rust coverage of visual-wasm: 100% by its own unit tests",
    "Rust coverage of alloc-counter: 100% by its own unit tests",
    "Rust coverage of fpenv: 100% by its own unit tests",
    "Rust coverage of audio-battery: 100% by its own unit tests",
    "Rust coverage of audio-testkit: 100% by its own unit tests",
    "Rust coverage of audio-cli: 100% by its own unit tests",
    "Rust coverage of audio-wasm: 100% by its own unit tests",
    "Rust coverage of coordinator: 100% by its own unit tests",
    "Rust coverage of web-page: 100% by its own unit tests",
    "Rust coverage of xtask: 100% by its own unit tests",
    "Rust coverage of the visual-cli binary: 100% by its tests",
    "Rust coverage of the visual-gpu binary: 100% by its tests",
    "Rust coverage of the audio-cli binary: 100% by its tests",
    "Rust coverage of the xtask binary: 100% by its tests",
    "Unsafe code under Miri",
    "Shared TypeScript policy (each rule proven to reject what it forbids): typecheck",
    "Shared TypeScript policy (each rule proven to reject what it forbids): lint",
    "Shared TypeScript policy (each rule proven to reject what it forbids): test",
    "Mutation testing of tooling/config (what changed since the last run)",
    "TypeScript coverage and mutation reports of tooling/config",
    "Every TypeScript source of tooling/config measured and mutated, or reviewed",
    "Fresh conformance export directory",
    "Conformance export (native WAV files)",
    "Conformance oracle (independent, TypeScript): typecheck",
    "Conformance oracle (independent, TypeScript): lint",
    "Conformance oracle (independent, TypeScript): test",
    "Conformance oracle (independent, TypeScript): oracle",
    "Mutation testing of tooling/oracle (what changed since the last run)",
    "TypeScript coverage and mutation reports of tooling/oracle",
    "Every TypeScript source of tooling/oracle measured and mutated, or reviewed",
    "The tool that builds the WebAssembly modules reproducibly (every WebAssembly package's): typecheck",
    "The tool that builds the WebAssembly modules reproducibly (every WebAssembly package's): lint",
    "The tool that builds the WebAssembly modules reproducibly (every WebAssembly package's): test",
    "Mutation testing of tooling/wasm-build (what changed since the last run)",
    "TypeScript coverage and mutation reports of tooling/wasm-build",
    "Every TypeScript source of tooling/wasm-build measured and mutated, or reviewed",
    "Engine WASM checks: build",
    "Engine WASM checks: typecheck",
    "Engine WASM checks: lint",
    "Engine WASM checks: test",
    "Engine WASM checks: benchmarks:check",
    "Engine WASM checks: test:allocation",
    "Engine WASM checks: test:reproducible",
    "Engine WASM checks: test:e2e",
    "Mutation testing of bindings/audio-wasm (what changed since the last run)",
    "TypeScript coverage and mutation reports of bindings/audio-wasm",
    "Every TypeScript source of bindings/audio-wasm measured and mutated, or reviewed",
    "Visual WASM checks (the same image hashes as the native renderer): build",
    "Visual WASM checks (the same image hashes as the native renderer): typecheck",
    "Visual WASM checks (the same image hashes as the native renderer): lint",
    "Visual WASM checks (the same image hashes as the native renderer): test",
    "Visual WASM checks (the same image hashes as the native renderer): test:reproducible",
    "Visual WASM checks (the same image hashes as the native renderer): benchmarks:check",
    "Mutation testing of bindings/visual-wasm (what changed since the last run)",
    "TypeScript coverage and mutation reports of bindings/visual-wasm",
    "Every TypeScript source of bindings/visual-wasm measured and mutated, or reviewed",
    "Build of bindings/visual-wasm, for the packages that use it",
    "Page module checks (the visual library and the coordinating layer in one module): build",
    "Page module checks (the visual library and the coordinating layer in one module): typecheck",
    "Page module checks (the visual library and the coordinating layer in one module): lint",
    "Page module checks (the visual library and the coordinating layer in one module): test",
    "Page module checks (the visual library and the coordinating layer in one module): test:reproducible",
    "Mutation testing of bindings/web-page (what changed since the last run)",
    "TypeScript coverage and mutation reports of bindings/web-page",
    "Every TypeScript source of bindings/web-page measured and mutated, or reviewed",
    "Build of bindings/audio-wasm, for the packages that use it",
    "Build of bindings/web-page, for the packages that use it",
    "Playground checks: typecheck",
    "Playground checks: lint",
    "Playground checks: test",
    "Playground checks: build",
    "Playground checks: test:e2e",
    "Mutation testing of apps/playground (what changed since the last run)",
    "TypeScript coverage and mutation reports of apps/playground",
    "Every TypeScript source of apps/playground measured and mutated, or reviewed",
    "Roadmap checks: typecheck",
    "Roadmap checks: lint",
    "Roadmap checks: test",
    "Roadmap checks: build",
    "Roadmap checks: test:e2e",
    "Mutation testing of apps/roadmap (what changed since the last run)",
    "TypeScript coverage and mutation reports of apps/roadmap",
    "Every TypeScript source of apps/roadmap measured and mutated, or reviewed",
    "Fresh Rust mutation report",
    "Rust mutation testing of the changes",
    "Rust unviable mutants reviewed",
    "Fresh visual corpus mutation report",
    "Visual corpus kills every mutant of the CPU reference's code that writes image bytes",
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
/// after its unviable gate and the visual corpus's mutation testing.
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
            Action::CorpusMutants(CHANGES_MODE),
            Action::UnviableReviewsCurrent,
            Action::MutationProofRecorded
        ],
        "after the unviable gate and the visual corpus's mutation testing"
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

/// The steps of a full check but its one test of every mutant of the GPU workspace, which
/// the default check runs within its run of the changes, when they reach it.
fn without_gpu_mutants(full: Vec<Step>) -> Vec<Step> {
    let gpu = full
        .iter()
        .filter(|step| matches!(step.action, Action::GpuMutants(Extent::Complete)))
        .count();
    assert_eq!(gpu, 1, "the GPU workspace's mutants, once, every one");
    full.into_iter()
        .filter(|step| !matches!(step.action, Action::GpuMutants(_)))
        .collect()
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
    let full = without_gpu_mutants(full);
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
            "Root: cargo mutants --no-config --output=. --cap-lints=true --gitignore=true --cargo-arg=--locked --cargo-arg=--config=profile.test.debug=false --cargo-arg=--config=target.'cfg(target_os = \"linux\")'.runner=[\"prlimit\", \"--as=4294967296\"] --test-tool=nextest --cargo-test-arg=--max-fail=1:immediate --cargo-test-arg=--no-tests=warn --workspace --jobs 3 --timeout-multiplier 15 --minimum-test-timeout 60 -- -- --include-ignored".to_owned(),
        ),
        (
            "unviable gate (Partial)".to_owned(),
            "unviable gate (Complete)".to_owned(),
        ),
        (
            "corpus mutants (Changes { jobs: 3 })".to_owned(),
            "corpus mutants (Full { jobs: 3 })".to_owned(),
        ),
    ];
    expected.extend(
        PACKAGES
            .iter()
            .map(|package| stryker(package.dir, " --force")),
    );
    assert_eq!(differences, expected, "only the mutation testing differs");
    let names: Vec<(String, String)> = changes
        .iter()
        .zip(&full)
        .map(|(default, every)| (default.name.clone(), every.name.clone()))
        .filter(|(default, every)| default != every)
        .collect();
    let mut expected_names = vec![(
        "Rust mutation testing of the changes".to_owned(),
        "Rust mutation testing".to_owned(),
    )];
    expected_names.extend(PACKAGES.iter().map(|package| {
        (
            format!(
                "Mutation testing of {} (what changed since the last run)",
                package.dir
            ),
            format!("Mutation testing of {} (every mutant)", package.dir),
        )
    }));
    assert_eq!(
        names, expected_names,
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

/// The reads the visual library adds to the default check: each crate's manifest by three
/// gates and its coverage report twice, each binary's coverage report twice, and each test
/// data reader and each audited unsafe file by nine gates.
fn visual_reads() -> usize {
    visual_crates()
        .saturating_mul(3 + 2)
        .saturating_add(visual_binaries().saturating_mul(2))
        .saturating_add(visual_crates_with_test_data().saturating_mul(9))
        .saturating_add(
            UNSAFE_FILES
                .iter()
                .filter(|file| file.contains("/visual-"))
                .count()
                .saturating_mul(9),
        )
}

/// What the packages beyond the five add to the default check's operations, as counted by
/// `runs_every_step_and_reports_success`: for each, one command per script of its checks and
/// 40 more (its Stryker run, its report directories emptied, the listing and the reads of
/// its manifest, sources, configurations and reports by the gates, its manifest read by the
/// part license and package check gates too); and 3 for the visual library in the browser
/// (its build output emptied, and its two entry points read in the bypass register).
fn added_package_operations() -> usize {
    crate::tests::packages_beyond_the_five()
        .map(|package| package.checks.len().saturating_add(40))
        .sum::<usize>()
        .saturating_add(3)
}

/// The reads of the shared constants gate in `fake`: each TypeScript source of a package, its
/// test code and the independent oracle apart, as the gate reads them.
fn shared_constants_reads(fake: &crate::tests::Fake) -> usize {
    PACKAGES
        .iter()
        .filter(|package| package.dir != "tooling/oracle")
        .map(|package| {
            fake.files
                .keys()
                .filter(|path| {
                    Path::new(path)
                        .extension()
                        .is_some_and(|extension| extension == "ts" || extension == "tsx")
                        && path
                            .strip_prefix(&format!("{}/", package.dir))
                            .is_some_and(|rel| !crate::reviews::is_test(rel))
                })
                .count()
        })
        .sum()
}

/// The visual crates whose tests read declared test data: the fake gives each a test data
/// reader, which nine gates read.
fn visual_crates_with_test_data() -> usize {
    crate::RUST_CRATES
        .iter()
        .filter(|krate| krate.name.starts_with("visual-") && !krate.test_data.is_empty())
        .count()
}

/// What the library lints gate, a gate of every Rust source read apart from the unsafe
/// gate, reads beyond the counts derived per gate in `runs_every_step_and_reports_success`:
/// each audited unsafe file of the audio library and each test data reader of a crate that
/// is not visual, the one counted there apart.
fn library_lints_reads() -> usize {
    let unsafe_files = UNSAFE_FILES
        .iter()
        .filter(|file| !file.contains("/visual-"))
        .count();
    let readers = crate::RUST_CRATES
        .iter()
        .filter(|krate| !krate.name.starts_with("visual-") && !krate.test_data.is_empty())
        .count();
    unsafe_files.saturating_add(readers).saturating_sub(1)
}

/// The files the conflict marker gate reads in a run of [`passing`]: the attributes file,
/// then every tracked text file, which is every file of the fake git would track
/// ([`crate::tests::tracked_in_fake`]) but the new Rust files and those declared binary.
fn conflict_gate_reads() -> usize {
    let fake = passing();
    let binary = crate::binary_suffixes(
        fake.files
            .get(crate::GIT_ATTRIBUTES)
            .expect("the attributes file"),
    )
    .expect("binary suffixes");
    let new = fake
        .files
        .get(&format!("capture {}", crate::tests::NEW_RUST_FILES.0))
        .cloned()
        .unwrap_or_default();
    let listed = fake
        .files
        .keys()
        .filter(|path| crate::tests::tracked_in_fake(path))
        .filter(|path| !new.split('\0').any(|file| file == path.as_str()))
        .filter(|path| !binary.iter().any(|suffix| path.ends_with(suffix.as_str())))
        .count();
    listed.checked_add(1).expect("a small count")
}

/// The operations of the retired patterns gate in a run of [`passing`]: its listing of the
/// tracked files, the reads the conflict marker gate makes of the attributes file and of
/// every tracked text file ([`conflict_gate_reads`]), the read of its list; and the read of
/// that list, a `.txt` file of the fake, by the citation gate, counted here.
fn retired_gate_operations() -> usize {
    conflict_gate_reads().checked_add(3).expect("a small count")
}

/// The operations of the list counts gate in a run of [`passing`]: its listing of the files
/// a sentence can be written in ([`crate::citation_sources`]), and the read of every file
/// it gives, as the fake lists them (the new Rust files apart); and the source kinds gate's
/// listing of the tracked files and read of the attributes file.
fn list_counts_and_kinds_operations() -> usize {
    let fake = passing();
    let new = fake
        .files
        .get(&format!("capture {}", crate::tests::NEW_RUST_FILES.0))
        .cloned()
        .unwrap_or_default();
    let read = fake
        .files
        .keys()
        .filter(|path| crate::tests::tracked_in_fake(path))
        .filter(|path| !new.split('\0').any(|file| file == path.as_str()))
        .filter(|path| {
            crate::citation_sources()
                .into_iter()
                .any(|pattern| crate::workspace::pathspec_matches(pattern, path))
        })
        .count();
    read.checked_add(3).expect("a small count")
}

/// The operations of the retired patterns, runnable hints, list counts, source kinds and
/// hermetic git gates in a run of [`passing`]: the last lists and reads the code files as the
/// hints gate does, the READMEs apart.
fn text_gate_operations() -> usize {
    [
        retired_gate_operations(),
        hint_gate_operations(),
        hint_gate_operations()
            .checked_sub(readme_reads().saturating_add(1))
            .expect("more reads than READMEs"),
        // The register names gate: the tracked files and the code files listed, the code
        // files read as the hermetic git gate reads them, and the two registers.
        hint_gate_operations()
            .checked_sub(readme_reads())
            .expect("more reads than READMEs")
            .saturating_add(2),
        list_counts_and_kinds_operations(),
    ]
    .into_iter()
    .sum()
}

/// The READMEs of the fake, which the runnable hints gate lists and reads.
fn readme_reads() -> usize {
    let fake = passing();
    fake.files
        .keys()
        .filter(|path| crate::tests::tracked_in_fake(path) && path.ends_with("README.md"))
        .count()
}

/// The operations of the runnable hints gate in a run of [`passing`]: its listings of the
/// code files and of the READMEs, and the read of every file they give: every code file of
/// the fake ([`crate::code_sources`]) but the new Rust files, as the fake lists them, and
/// every README of the fake.
fn hint_gate_operations() -> usize {
    let fake = passing();
    let new = fake
        .files
        .get(&format!("capture {}", crate::tests::NEW_RUST_FILES.0))
        .cloned()
        .unwrap_or_default();
    let listed = fake
        .files
        .keys()
        .filter(|path| crate::tests::tracked_in_fake(path))
        .filter(|path| !new.split('\0').any(|file| file == path.as_str()))
        .filter(|path| {
            crate::code_sources()
                .iter()
                .filter_map(|pattern| pattern.split_once('*'))
                .any(|(prefix, suffix)| path.starts_with(prefix) && path.ends_with(suffix))
                || path.ends_with("README.md")
        })
        .count();
    listed.checked_add(2).expect("a small count")
}

/// What a default run reads for the order of the tests in mutant runs: each workspace's
/// nextest configuration, read by the Cargo configuration gate (after its listing, the
/// thirtieth) and by the citation gate, and the baseline log of the root's mutation run.
fn nextest_reads() -> usize {
    crate::CargoWorkspace::ALL
        .len()
        .checked_mul(2)
        .and_then(|reads| reads.checked_add(1))
        .expect("a small count")
}

/// What the GPU workspace and its crate add to a default run: its formatting, lints,
/// documentation, tests, supply chain and time benchmark (six commands), its metadata
/// beside the root's for each of the six gates reading the crates, its manifest, its policy
/// and the root's manifest read by the gates that check them (17, counted on a run,
/// 2026-11-16); its two builds the loaded-files gate reads, the code's and the tests' (3,
/// counted on a run, 2026-11-19); and its shader test, read by the loaded-files gate for
/// the shaders it validates and by the nine gates of every Rust source (1 + 9).
/// Roadmap #24 step 107 derives each count from the registries.
const fn gpu_operations() -> usize {
    17 + 3 + 1 + 9
}

/// What the page adds to a default run: the coordinating layer's crate and the page
/// module, each its coverage measured (a clean and a measurement) and its manifest and
/// report read, as every crate's (7 each); the page module's audited unsafe file read by
/// eight gates; and the page module's package, beyond what each added package adds, its
/// build output emptied and its sources read by the gates on TypeScript sources (5;
/// counted on a run, 2026-11-17; roadmap #24 step 107 derives each count from the
/// registries).
const fn page_operations() -> usize {
    7 + 7 + 8 + 5
}

/// The reads [`counted_reads`] holds of files git ignores, from when the fake listed every
/// file it holds as tracked: the citation gate's read of each coverage report a passing run
/// holds under `target/` (a `.json` file), and of the spent record the run writes there.
fn ignored_reads() -> usize {
    passing()
        .files
        .keys()
        .filter(|path| {
            path.starts_with("target/")
                && Path::new(path.as_str())
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
        })
        .count()
        .checked_add(1)
        .expect("a small count")
}

/// The files a passing check reads, as first counted step by step, before the reads the
/// later counts name one by one; without the reads of files git ignores
/// ([`ignored_reads`]), which those first counts held.
fn counted_reads() -> usize {
    (9_usize + 2
        + 13
        + 7
        + 15
        + 3
        + 7
        + 21
        + 3
        + 1
        + 7
        + 3
        + 13
        + 10
        + 21
        + 8
        + 2
        + 13
        + 40
        + 20
        + 3
        + 3 * 9
        + 3 * 12
        + 1
        + 1
        + 6
        + 3
        + 93
        + 5
        // The corpus's exemptions, read by the names gate as the unviable reviews are.
        + 1)
    .checked_sub(ignored_reads())
    .expect("fewer reads ignored than counted")
}

#[test]
fn runs_every_step_and_reports_success() {
    let mut fake = passing();
    let constants = shared_constants_reads(&fake);
    let mut out = Vec::new();
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut fake, &mut out),
        Ok(()),
        "all green"
    );
    // The fifty-sixth capture is the breaking rule's listing of the history, the
    // fifty-seventh the GPU workspace's list of mutants, beside the root's, the fifty-eighth
    // the proof record's version of the mutants' test runner, the last two the trees of the
    // CPU reference in each workspace, then each workspace's resolved metadata for the
    // build-time code gate. The refusal of a mutant left in place captures its search. The
    // validated shaders gate lists the sources of the crates holding a shader, the README
    // gate the specifications, and so does the gate of the names reviews and specifications
    // cite, which also captures the merge base with main and the reviews there, and lists the
    // TypeScript files, whose tests the index of the specifications names; the linked
    // conformance gate lists the packages' crates and TypeScript files, and the shared
    // constants gate the TypeScript files, and so does the module table gate. The visual
    // corpus's mutation step empties its report, then captures the sources of the rendering
    // code's crates, the merge base, the files changed since and the untracked files, in two
    // listings (those no ignore names, and the code, test data and settings), which reach none
    // of its inputs. The mutation step and the proof record each capture the index for the
    // bytes git shows, and the proof record the untracked code, test data and settings. Each
    // of the three readings of the tree's state (before the checks, after them, and the proof
    // record's) captures the tracked files with their tags.
    let (tools, emptied, listed, captured, written) = (crate::TOOLS.len(), 16, 37, 76, 4);
    // Three of the commands are the doc tests' with their error codes checked: the
    // fixture's, then each workspace's; one per package another uses is its build, prepared
    // for them.
    let commands = 82 + 2 * (visual_crates() + visual_binaries()) + crate::needed_builds().len();
    let read = counted_reads()
        // The license step reads the pnpm workspace configuration, whose register of
        // `pnpm audit`'s exceptions it holds against what the apps ship.
        + 1
        // It reads the two license statements, README.md and CLAUDE.md, and CLAUDE.md, a
        // tracked Markdown file of the fake, is read by two gates of every text file too;
        // so are the corpus step's review file and report, held by the fake, whose reads its
        // files do not count already (counted on a run, 2026-11-20).
        + 2
        + 4
        // fpenv, beside the libraries, reads no test data and audio-host does, as does
        // audio-wasm: one test data reader more than these counts were written for, which
        // nine gates read (roadmap #24 step 107 derives each count from the registries).
        + 9
        + library_lints_reads()
        + conflict_gate_reads()
        + text_gate_operations()
        + visual_reads()
        + added_package_operations()
        // Each package's TypeScript sources, read by the shared constants gate.
        + constants
        // Each file the math layer gate names, held by the fake as the gate requires it,
        // read by ten gates of Rust sources besides the conflict marker gate's read above.
        + 10 * crate::MATH_LAYER_NAMED.len()
        // Each other root of the audio library, which the fake holds for the lints its
        // roots deny, read by the nine gates of every Rust source but the math layer's.
        + 9 * crate::RUST_CRATES
            .iter()
            .filter(|krate| krate.roles.contains(&crate::CrateRole::Library))
            .filter(|krate| {
                !crate::MATH_LAYER_NAMED.contains(&format!("{}/src/lib.rs", krate.dir).as_str())
            })
            .count()
        + nextest_reads()
        + gpu_operations()
        // Each root of the visual library in the root workspace, which the fake holds for
        // the lints its roots deny, read by the nine gates of every Rust source (the GPU
        // renderer's root the fake holds already, among its workspace's sources).
        + 9 * crate::RUST_CRATES
            .iter()
            .filter(|krate| {
                krate.roles.contains(&crate::CrateRole::VisualLibrary)
                    && krate.workspace == crate::CargoWorkspace::Root
            })
            .count()
        // The root of each `no_std` crate not built for every portability target, which the
        // fake holds for the portability gate, read by the nine gates of every Rust source.
        + 9 * crate::NO_STD_NOT_PORTABLE.len()
        // The fake's `.gitattributes`, read by the citation gate, as every kind of file a
        // sentence can be written in (`citation_sources`).
        + 1
        // The build-time code register, read by its gate and by two gates reading every
        // tracked text file, one of which the count of the fake's files above already
        // follows (counted on a run, 2026-11-19).
        + 2
        + page_operations()
        // The crate of each TypeScript package that has one, read by the linked conformance
        // gate (the fake holds no test of theirs).
        + PACKAGES
            .iter()
            .filter(|package| crate::RUST_CRATES.iter().any(|krate| krate.dir == package.dir))
            .count()
        // The root of each crate holding a validated shader, the one source of it the fake
        // holds, read by the validated shaders gate for the shader's constant.
        + crate::RUST_CRATES
            .iter()
            .filter(|krate| {
                crate::loaded_files::VALIDATED_SHADERS
                    .iter()
                    .any(|shader| krate.holds(shader).is_some())
            })
            .count();
    assert_eq!(
        fake.ran.len(),
        tools + commands + emptied + listed + captured + written + read,
        "every operation of a default run, by kind: the tools the preflight asks, the \
         commands run, the directories emptied, what git lists, what is captured, the files \
         written and the files read, each count derived above from the registries it \
         follows (the crates, the packages, the unsafe files, the steps), so that no number \
         written here can go stale"
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
    // The GPU workspace's formatting, lints, documentation, tests, supply chain, CPU
    // reference and time benchmark: seven; the coverage of the coordinating layer and of
    // the page module: two; the builds prepared beyond the engine's.
    // And the build-time code gate.
    // And the refusal of a mutant left in place.
    // And the retired patterns and runnable hints gates.
    // And the visual corpus's mutation testing: its report emptied, then its run.
    // And the source kinds, list counts, hermetic git and register names gates.
    let checks =
        117 + visual_steps() + added_package_steps() + 7 + 2 + added_builds() + 1 + 1 + 2 + 2 + 4;
    assert!(
        text.starts_with(&format!("▶ [1/{checks}] Tools and platform\n")),
        "numbered progress: {text}"
    );
    assert!(
        text.contains(&format!(
            "▶ [{}/{checks}] Roadmap checks: test:e2e\n  done in 1.5 s\n",
            // The last package: then its Stryker run and its two gates, the Rust mutation
            // testing's five steps, the visual corpus's two and the tree checked unchanged.
            checks - 11
        )),
        "the roadmap's step announced: {text}"
    );
    assert!(
        text.ends_with(&format!(
            "  done in 1.5 s\n✔ all {checks} checks passed in {}\n",
            elapsed(checks)
        )),
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
        env: Vec::new(),
    };
    assert_eq!(cmd.line(), "cargo fmt --check", "program and arguments");
    let bare = Cmd {
        program: "pnpm",
        args: vec![],
        dir: Dir::Root,
        env: Vec::new(),
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
        // Refused first by the GPU workspace's mutation testing, which adds to that report.
        failed(format!(
            "Rust mutation testing (GPU workspace): cannot read {UNVIABLE_REPORT}: missing"
        )),
        "a full run always writes its report"
    );
    let mut no_reviews = passing();
    let removed_reviews = no_reviews.files.remove(UNVIABLE_REVIEWED);
    assert!(removed_reviews.is_some(), "reviews removed");
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut no_reviews, &mut Vec::new()),
        failed(format!(
            "Every function a review or a specification names exists, each new review naming its test; each reader of a file only a tool writes is named in the index: cannot read \
             {UNVIABLE_REVIEWED}: missing"
        )),
        "missing reviews"
    );
    let mut unreviewed = passing();
    let _previous = unreviewed
        .files
        .insert(UNVIABLE_REVIEWED.to_owned(), String::new());
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut unreviewed, &mut Vec::new())
            .map_err(|failure| failure.summary),
        Err("Rust unviable mutants reviewed: unviable mutants must be reviewed in mutants-unviable.txt: not reviewed: crates/a/src/b.rs: replace f -> T with Default::default()".to_owned()),
        "unreviewed mutant"
    );
}

#[test]
fn fails_when_equivalent_files_cannot_be_read() {
    let mut no_list = passing();
    no_list.broken = Some(format!("list {}", crate::stryker_sources().join(" ")));
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut no_list, &mut Vec::new()),
        failed(format!(
            "TypeScript equivalent mutants reviewed: cannot list the TypeScript files: `git \
             ls-files -- {}` exited with code 128: fatal: not a git repository",
            crate::stryker_sources().join(" ")
        )),
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
fn reads_a_disable_comment_in_every_kind_stryker_mutates() {
    // Findings H7-3, H12-1 and H17-2 of the fifth deep review of visual block 2 (#34): the
    // equivalent gate read `.ts` and `.tsx` files only, so a disable comment in a `.mts` or
    // a `.cjs` source, which Stryker mutates, needed no review.
    let step = [Step {
        name: "Equivalent".to_owned(),
        action: Action::EquivalentGate,
    }];
    for path in ["tooling/config/x.mts", "tooling/config/x.cjs"] {
        let mut fake = passing();
        let added = fake
            .files
            .insert(path.to_owned(), crate::tests::DISABLED_SOURCE.1.to_owned());
        assert!(added.is_none(), "fresh fake");
        assert_eq!(
            run_steps(&step, &mut fake, &mut Vec::new()),
            failed(format!(
                "Equivalent: disabled mutants must be reviewed in mutants-equivalent.txt: not \
                 reviewed: {path}: EqualityOperator at while (a < 2) {{}}"
            ))
        );
    }
}

#[test]
fn release_gate_runs_on_the_real_files() {
    let mut fake = passing();
    let replaced = fake.files.insert(CHANGELOG.to_owned(), "nope".to_owned());
    assert!(replaced.is_some(), "changelog replaced");
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut fake, &mut Vec::new())
            .map_err(|failure| failure.summary),
        Err("One version everywhere, changelog well formed, released sections as on main: the versions and the changelog: 2 problems, listed above".to_owned()),
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
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut unlisted, &mut Vec::new())
            .map_err(|failure| failure.summary),
        Err("Packages and crates known to the checks: cannot list the *package.json files: `git ls-files -- *package.json` exited with code 128: fatal: not a git repository".to_owned()),
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
        | Action::ListCountsGate
        | Action::ConflictMarkerGate
        | Action::SourceKindsGate
        | Action::RetiredPatternsGate
        | Action::RunnableHintsGate
        | Action::HermeticGitGate
        | Action::RegisterNamesGate
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
        | Action::PackageCheckGate
        | Action::SupplyChainGate
        | Action::CpuReferenceGate
        | Action::BuildTimeGate
        | Action::TypeScriptMutationGate { .. }
        | Action::TypeScriptReportsGate(_)
        | Action::TypeScriptScopeGate(_)
        | Action::UnsafeGate
        | Action::LibraryLintsGate
        | Action::UnviableGate(_)
        | Action::UnviableReviewsCurrent
        | Action::ShardsUnviableGate(_)
        | Action::VersionGate
        | Action::VocabularyGate
        | Action::WorkflowGate
        | Action::Run(_)
        | Action::ToolGate(_)
        | Action::RustMutants(_)
        | Action::Coverage(_)
        | Action::CoverageLocality
        | Action::GpuMutants(_)
        | Action::CorpusMutants(_)
        | Action::RustMutantsOfChanges { .. }
        | Action::RustMutantsOfChangedLines { .. }
        | Action::Bless(_)
        | Action::Conformance
        | Action::MutationProofRecorded
        | Action::FreshDirs(_)
        | Action::FusedBuild
        | Action::DocTests
        | Action::TreeRecorded
        | Action::TreeUnchanged
        | Action::MutantLeftGate
        | Action::Preflight(_) => false,
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

/// The summary of a refusal of the list counts gate: a sentence counting a typed list.
fn list_counts_refusals() -> Vec<String> {
    let counted = [(
        "a.rs".to_owned(),
        "const PAIR: [&str; 2] = [\"a\", \"b\"];\n/// PAIR holds 2 names.\n".to_owned(),
    )];
    vec![
        crate::list_counts_gate(&counted)
            .expect_err("a count")
            .summary,
    ]
}

/// The summary of a refusal of the hermetic git gate: a process no row lists.
fn hermetic_git_refusals() -> Vec<String> {
    let bare = [(
        "a_tests.rs".to_owned(),
        "fn f() { Command::new(\"git\"); }".to_owned(),
    )];
    vec![
        crate::hermetic_git_gate(&bare)
            .expect_err("an unlisted start")
            .summary,
    ]
}

/// The summary of a refusal of the register names gate: a reason citing a file gone.
fn register_names_refusals() -> Vec<String> {
    let register = [("r.txt", "a.ts: x -- see gone/b.ts\n")];
    vec![
        crate::register_names_gate(
            &register,
            &alloc::collections::BTreeSet::new(),
            &alloc::collections::BTreeSet::new(),
        )
        .expect_err("a file gone")
        .summary,
    ]
}

/// The summary of a refusal of the source kinds gate.
fn source_kinds_refusals() -> Vec<String> {
    vec![
        crate::source_kinds_gate(&["Makefile".to_owned()])
            .expect_err("no kind")
            .summary,
    ]
}

/// The retired patterns gate refusing each kind of problem it reports: a pattern held, a
/// line not of the list's form, a path that holds no source.
fn retired_refusals() -> Vec<String> {
    let held = [("a.rs".to_owned(), "weak\n".to_owned())];
    [
        crate::retired_patterns_gate(&held, "retire `weak` -- r\n"),
        crate::retired_patterns_gate(&[], "retire\n"),
        crate::retired_patterns_gate(&held, "retire `x` in b/ -- r; kept elsewhere: w\n"),
    ]
    .into_iter()
    .map(|result| result.expect_err("refused").summary)
    .collect()
}

/// The runnable hints gate refusing each kind of problem it reports: a hint naming no
/// command, a hint opened in another form, a README's command not runnable and a README's
/// hint naming none.
fn hint_refusals() -> Vec<String> {
    let (see, commands) = (crate::HINT_OPENING, crate::README_HINT_OPENING);
    [
        ("a.rs", format!("\"{see}x help)\"")),
        ("a.rs", format!("\"{commands}`x`)\"")),
        ("README.md", format!("{commands}`x`)")),
        ("README.md", format!("{commands}x)")),
        ("a.rs", format!("\"{see}cargo xtask <name>)\"")),
        ("a.rs", format!("\"{see}git restore --staged -- :/a.rs)\"")),
        ("a.rs", format!("\"{see}git checkout HEAD -- a.rs)\"")),
        ("a.rs", "\"run `cargo xtask help`\"".to_owned()),
    ]
    .into_iter()
    .map(|(path, text)| {
        crate::runnable_hints_gate(&[(path.to_owned(), text)])
            .expect_err("refused")
            .summary
    })
    .collect()
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
        (Action::ListCountsGate, list_counts_refusals()),
        (
            Action::ConflictMarkerGate,
            vec![
                refused(crate::conflict_marker_gate(&[(
                    "a.md".to_owned(),
                    "<<<<<<< x\n".to_owned(),
                )])),
                refused(crate::binary_suffixes("a binary\n").map(|_| ())),
            ],
        ),
        (Action::SourceKindsGate, source_kinds_refusals()),
        (Action::RetiredPatternsGate, retired_refusals()),
        (Action::RunnableHintsGate, hint_refusals()),
        (Action::HermeticGitGate, hermetic_git_refusals()),
        (Action::RegisterNamesGate, register_names_refusals()),
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
    assert_eq!(
        times_named(&step.name, &summary),
        step.name.split("; ").count(),
        "each clause once: {summary}"
    );
    assert_eq!(
        summary,
        format!(
            "{}: crates/audio-core/src/a.rs:1: names `test` outside test code: tests live in tests/ or in a *_tests.rs file declared `#[cfg(test)] #[path = \"<name>_tests.rs\"] mod <module>;`",
            step.name
        )
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
        Err(Failure {
            summary: "Commit messages, and a changelog entry for each change of behavior: the \
                      history gate is pinned to commits main does not hold: 2 problems, listed \
                      above"
                .to_owned(),
            details: ["SCOPES_SINCE", "BREAKING_SINCE"]
                .map(|name| {
                    format!(
                        "{name} names 8ff0722ec0273e176d23865891563e8bbe8a1c41, which is not in \
                         the history of fed4560000000000000000000000000000000000 (merge base \
                         with origin/main): pin a commit main holds"
                    )
                })
                .to_vec(),
            usage: false,
        }),
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
        let checked: Vec<Option<usize>> = package
            .checks
            .iter()
            .map(|script| {
                steps.iter().position(|step| {
                    step.name == format!("{}: {script}", package.step)
                        && step.action
                            == Action::Run(Cmd {
                                program: "pnpm",
                                args: vec!["run".to_owned(), (*script).to_owned()],
                                dir: Dir::Package(package.dir),
                                env: Vec::new(),
                            })
                })
            })
            .collect();
        let gated = steps
            .iter()
            .position(|step| step.action == Action::TypeScriptReportsGate(package));
        assert!(
            !checked.is_empty()
                && checked.iter().all(Option::is_some)
                && checked.windows(2).all(|pair| pair[0] < pair[1])
                && checked.last().copied().flatten() < gated,
            "{}: each of its checks runs, in order, before the reports gate: {checked:?}",
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
        vec!["visual-wasm", "alloc-counter", "audio-wasm", "web-page"],
        "the crates of the audited files, in the order of RUST_CRATES, but the assembly one"
    );
    assert_eq!(
        crate::ASSEMBLY_FILES,
        ["crates/fpenv/src/word.rs"],
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
            "cargo +nightly-2026-11-01 {} test --package visual-wasm --package \
             alloc-counter --package \
             audio-wasm --package web-page --locked -- --include-ignored",
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
    let clean = |line: &str| line.contains("llvm-cov clean --workspace");
    let measurements: Vec<usize> = fake
        .ran
        .iter()
        .enumerate()
        .filter(|(_, line)| line.contains("llvm-cov --manifest-path"))
        .map(|(index, _)| index)
        .collect();
    assert_eq!(
        measurements.len(),
        // The coordinating layer's crate and the page module: two more.
        14 + visual_crates() + visual_binaries() + 1 + 2,
        "every crate and binary measured, and the locality fixture"
    );
    for index in measurements {
        assert!(
            fake.ran
                .get(index.wrapping_sub(1))
                .is_some_and(|line| clean(line)),
            "a measurement right after a clean coverage build"
        );
    }
}

#[test]
fn stops_when_the_coverage_build_cannot_be_emptied() {
    let mut fake = passing();
    fake.broken = Some(
        Coverage {
            krate: crate::RUST_CRATES[0],
            measured: crate::Measured::Library,
        }
        .clean_command()
        .line(),
    );
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut fake, &mut Vec::new()),
        failed(
            "Rust coverage counts a crate's own unit tests only: cannot run `cargo +nightly-2026-11-01 llvm-cov clean --workspace`: not found"
        ),
        "the reason the clean failed"
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
    let steps: Vec<Step> = check_steps(CHANGES_MODE)
        .into_iter()
        .filter(|step| step.action == Action::SupplyChainGate)
        .collect();
    assert_eq!(steps.len(), 1, "one step");
    for path in crate::SUPPLY_CHAIN_FILES {
        let mut fake = passing();
        let _removed = fake.files.remove(path);
        assert_eq!(
            run_steps(&steps, &mut fake, &mut Vec::new()),
            failed(format!(
                "Supply-chain configuration as xtask defines it: cannot read {path}: missing"
            )),
            "names the file"
        );
    }
    let mut fake = passing();
    let _npmrc = fake.files.insert("apps/x/.npmrc".to_owned(), String::new());
    assert_eq!(
        run_steps(&steps, &mut fake, &mut Vec::new()),
        failed(
            "Supply-chain configuration as xtask defines it: the supply-chain configuration: apps/x/.npmrc: an npm configuration file, from which pnpm reads settings (`minimum-release-age-exclude` …): they go in pnpm-workspace.yaml, whose keys xtask checks"
        ),
        "the tracked npm configuration files"
    );
    for hooks in [".pnpmfile.cjs", "apps/x/.pnpmfile.mjs"] {
        let mut hooked = passing();
        let _hooks = hooked.files.insert(hooks.to_owned(), String::new());
        assert_eq!(
            run_steps(&steps, &mut hooked, &mut Vec::new()),
            failed(format!(
                "Supply-chain configuration as xtask defines it: the supply-chain configuration: {hooks}: a pnpm hooks file, whose code pnpm runs at every install, rewriting the manifests it resolves (`readPackage`) and its settings (`updateConfig`): a setting goes in pnpm-workspace.yaml, whose keys xtask checks"
            )),
            "the tracked pnpm hooks files"
        );
    }
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
    assert_eq!(
        run_steps(&steps, &mut broken, &mut Vec::new()),
        failed(format!(
            "Boxes unchanged under the math layer's fused multiply-add option (a test build): the fused test build failed: `{}` exited with code 1: failed",
            crate::fused_command().line()
        )),
        "a build that failed"
    );
    let mut silent = passing();
    let _replaced = silent.files.insert(
        format!("capture {}", crate::fused_command().line()),
        "test result: ok.\n".to_owned(),
    );
    assert_eq!(
        run_steps(&steps, &mut silent, &mut Vec::new()).map_err(|refused| refused.summary),
        Err("Boxes unchanged under the math layer's fused multiply-add option (a test build): the fused test build ran without its option: `test math::tests::is_built_with_the_fused_multiply_add_option ... ok` is missing from its output".to_owned()),
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
    let _removed = fake.files.remove(&format!(
        "capture {}",
        crate::metadata_command(crate::CargoWorkspace::Root).line()
    ));
    assert_eq!(
        run_steps(&steps, &mut fake, &mut Vec::new()),
        failed(
            "Box arithmetic only through the math layer: libm in audio-core and audio-testkit only, no raw float in box code, no iterator reduction outside the math layer: cannot list the crates: `cargo metadata --format-version 1 --no-deps --locked` exited with code 1: failed"
        ),
        "says what it could not do"
    );
    assert_eq!(
        run_steps(&steps, &mut passing(), &mut Vec::new()),
        Ok(()),
        "the fake's metadata has no libm"
    );
}

/// The shared known-answer vectors of the escaping of untrusted text.
const SAFE_VECTORS: &str = include_str!("../../../spec/safe-message-vectors.json");

/// The text of a list of `[piece, times]`.
fn vector_text(pieces: &serde_json::Value) -> String {
    pieces
        .as_array()
        .expect("a list of pieces")
        .iter()
        .map(|piece| {
            let times = usize::try_from(piece[1].as_u64().expect("a count")).expect("usize");
            piece[0].as_str().expect("a piece of text").repeat(times)
        })
        .collect()
}

#[test]
fn reproduces_the_shared_vectors_of_escaped_messages() {
    // Roadmap #24 step 55: xtask's copy was tied to the other tools' by no test.
    let json: serde_json::Value = serde_json::from_str(SAFE_VECTORS).expect("valid JSON");
    assert_eq!(
        json["max_chars"].as_u64(),
        Some(u64::try_from(crate::MAX_MESSAGE_CHARS).expect("u64")),
        "same limit"
    );
    let vectors = json["vectors"].as_array().expect("a list of vectors");
    assert_eq!(
        vectors.len(),
        9,
        "every vector is read: the nine of the file"
    );
    for vector in vectors {
        assert_eq!(
            crate::safe_message(&vector_text(&vector["input"])),
            vector_text(&vector["output"]),
            "{}",
            vector["name"]
        );
    }
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
        "other text is kept, the neighbors of the escaped ranges included"
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

#[test]
fn bounds_a_reason_before_its_remedy() {
    // Finding MC-1 of the seventh deep review of visual block 2 (#34): a remedy added before
    // the line was bounded was the part a long reason cut.
    use crate::MAX_MESSAGE_CHARS;
    use crate::steps::{CONTEXT_ROOM, remedied};
    let room = |left: usize| "r".repeat(MAX_MESSAGE_CHARS - CONTEXT_ROOM - left);
    // Finding R-3 of the mini review of the eighth: a room of 0 emptied the reason, one of 1
    // left the mark alone; a reason is never said empty, so either keeps it whole.
    assert_eq!(
        remedied("ab", &room(0)),
        format!("ab{}", room(0)),
        "no room: the reason whole"
    );
    assert_eq!(
        remedied("abc", &room(1)),
        format!("abc{}", room(1)),
        "a room of the mark alone: the reason whole"
    );
    assert_eq!(
        remedied("ab", &room(2)),
        format!("ab{}", room(2)),
        "exactly its room"
    );
    assert_eq!(
        remedied("\n", &room(5)),
        format!("\\u{{a}}{}", room(5)),
        "an escape counted whole"
    );
    let over = format!("{}r", room(0));
    assert_eq!(
        remedied("ab", &over),
        format!("ab{over}"),
        "a remedy past its room"
    );
}

#[test]
fn keeps_every_remedy_of_the_tree_whole_in_its_longest_context() {
    // Finding MC-1 of the seventh deep review of visual block 2 (#34): each reason said of the
    // tree, in the pre-push hook or the mutation preflight, after the longest step's name.
    use crate::steps::CONTEXT_ROOM;
    use crate::{MAX_MESSAGE_CHARS, safe_message};
    let step = check_steps(Mode::Full { jobs: FAKE_JOBS })
        .into_iter()
        .map(|step| step.name)
        .max_by_key(|name| name.chars().count())
        .expect("a step");
    // Paths long enough to cut each reason: two of the remedies name theirs twice.
    let path = "d/".repeat(84) + "f";
    let long = "d/".repeat(150) + "f";
    let tree = |untracked: Vec<String>, listed: String, unshown: Option<String>| crate::TreeView {
        status: String::new(),
        untracked,
        listed,
        filtered: None,
        unshown,
    };
    let pushed =
        |view: crate::TreeView| crate::pre_push_gate("the push cannot be checked", "1", &view, &[]);
    let pre_push = "the push cannot be checked: the working tree may differ from what is \
                    committed, which the check would include: ";
    // Each reason whole, split where its remedy starts.
    let split = |whole: String, at: &str| {
        let (reason, remedy) = whole.split_once(at).expect("a remedy");
        (format!("{reason}{at}"), remedy.to_owned())
    };
    let clear = |flag: &str| crate::tests::cleared(flag, &path);
    for (failure, context, (reason, remedy)) in [
        (
            pushed(tree(vec![long.clone()], String::new(), None)),
            pre_push,
            split(crate::tests::untracked_reason(&long, &long), "would read: "),
        ),
        (
            pushed(tree(Vec::new(), format!("s {path}\0"), None)),
            pre_push,
            split(
                crate::tests::hidden_reason(
                    &path,
                    &format!(
                        "{} then {}",
                        clear("--no-assume-unchanged"),
                        clear("--no-skip-worktree")
                    ),
                ),
                "changes: ",
            ),
        ),
        (
            pushed(tree(
                Vec::new(),
                String::new(),
                crate::unshown(&crate::TreeBytes {
                    staged: format!("100644 1 0\t{path}\0"),
                    deleted: String::new(),
                    raw: "2\n".to_owned(),
                    converted: "2\n".to_owned(),
                    changed: String::new(),
                }),
            )),
            pre_push,
            split(crate::tests::unshown_reason(&path, &path), "committed: "),
        ),
        (
            crate::preflight::mutant_left_gate(&format!("{long}:1:x\n")),
            "a mutant left in place by a stopped mutation run (each file named restores from \
             the index, which keeps what you staged, by the hint after it): ",
            split(
                crate::tests::restore(&format!("{long}:1"), &long),
                "marker: ",
            ),
        ),
    ] {
        let room = MAX_MESSAGE_CHARS - CONTEXT_ROOM - remedy.chars().count();
        let cut = reason.chars().take(room - 1).collect::<String>();
        assert_eq!(
            failure.map_err(|failure| safe_message(&format!("{step}: {}", failure.summary))),
            Err(format!("{step}: {context}{cut}…{remedy}")),
            "{remedy}"
        );
    }
    // Finding PB-1 of the eighth deep review of visual block 2 (#34): from a path of 205
    // characters (R-3 of its mini review: 205 emptied the reason), the remedy of a file hidden
    // by both marks leaves its reason no room, so the reason stays whole and the line is cut
    // with its remedy (#24 step 109).
    for edge in ["d/".repeat(102) + "f", "d/".repeat(102) + "ff"] {
        let both = [
            crate::tests::cleared("--no-assume-unchanged", &edge),
            crate::tests::cleared("--no-skip-worktree", &edge),
        ];
        assert_eq!(
            pushed(tree(Vec::new(), format!("s {edge}\0"), None))
                .map_err(|failure| failure.summary),
            Err(format!(
                "{pre_push}{}",
                crate::tests::hidden_reason(&edge, &both.join(" then "))
            )),
            "the reason whole at the edge, its remedy after it"
        );
    }
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

#[test]
fn limits_the_memory_of_every_mutant_test_program_on_linux_only() {
    assert_eq!(crate::MUTANT_MEMORY_LIMIT, 4_294_967_296, "4 GiB");
    assert_eq!(
        crate::MUTANT_TEST_RUNNER,
        "--cargo-arg=--config=target.'cfg(target_os = \"linux\")'.runner=[\"prlimit\", \
         \"--as=4294967296\"]",
        "prlimit's address-space limit, the test programs of Linux only"
    );
    assert!(
        crate::MUTANT_TEST_RUNNER.ends_with(&format!("{}\"]", crate::MUTANT_MEMORY_LIMIT)),
        "the runner holds the limit the constant names"
    );
    for cmd in [
        crate::changes_mutants_command(FAKE_JOBS),
        crate::workspace_mutants_command(FAKE_JOBS, None),
    ] {
        assert!(
            cmd.args.iter().any(|arg| arg == crate::MUTANT_TEST_RUNNER),
            "every cargo-mutants run: {}",
            cmd.line()
        );
    }
}

#[test]
fn runs_every_mutant_s_tests_under_nextest_stopping_at_the_first_failure() {
    assert_eq!(crate::MUTANT_TEST_TOOL, "--test-tool=nextest", "nextest");
    assert_eq!(
        crate::MUTANT_FAIL_FAST,
        "--cargo-test-arg=--max-fail=1:immediate",
        "the first failure stops the run and terminates the tests still running, given to \
         the test runs only (nextest refuses it with --no-run)"
    );
    assert_eq!(
        crate::MUTANT_NO_TESTS,
        "--cargo-test-arg=--no-tests=warn",
        "a run of no test missed, as under cargo test, not caught by nextest's exit code 4"
    );
    assert!(
        crate::PINNED_TOOLS.contains(&("cargo-nextest", "0.9.152")),
        "the runner pinned, past the advisory of 0.9.125 to 0.9.149"
    );
    for cmd in [
        crate::changes_mutants_command(FAKE_JOBS),
        crate::workspace_mutants_command(FAKE_JOBS, None),
        crate::gpu_mutants_command(),
    ] {
        for arg in [
            crate::MUTANT_TEST_TOOL,
            crate::MUTANT_FAIL_FAST,
            crate::MUTANT_NO_TESTS,
        ] {
            assert!(
                cmd.args.iter().any(|given| given == arg),
                "{arg} in every cargo-mutants run: {}",
                cmd.line()
            );
        }
        assert!(
            cmd.args.ends_with(&[
                "--".to_owned(),
                "--".to_owned(),
                crate::INCLUDE_IGNORED.to_owned()
            ]),
            "the ignored tests still run, through nextest's own reading of the harness \
             argument: {}",
            cmd.line()
        );
    }
}

#[test]
fn matches_a_mutant_by_its_whole_name_every_regex_character_escaped() {
    assert_eq!(
        crate::steps::REGEX_META,
        "\\.+*?()|[]{}^$#&-~",
        "the meta characters of Rust's regex (regex_syntax::is_meta_character)"
    );
    assert_eq!(
        crate::steps::mutant_name_re(
            "crates/a/src/b.rs:61:5: replace engine_block with () in <impl Add for Sample>"
        ),
        "^crates/a/src/b\\.rs:61:5: replace engine_block with \\(\\) in <impl Add for Sample>$",
        "anchored; `<`, `>`, `/` and `:` left as they are (`\\<` is a word boundary)"
    );
    let every: String = crate::steps::REGEX_META.chars().collect();
    let escaped: String = every
        .chars()
        .flat_map(|character| ['\\', character])
        .collect();
    assert_eq!(
        crate::steps::mutant_name_re(&every),
        format!("^{escaped}$"),
        "each meta character escaped once"
    );
    assert_eq!(
        crate::steps::mutant_name_re(""),
        "^$",
        "nothing but the anchors"
    );
}

#[test]
fn tests_again_with_the_doc_tests_exactly_the_mutants_a_run_missed() {
    let missed = [
        "crates/audio-core/src/block.rs:61:5: replace engine_block with ()",
        "src/lib.rs:3:9: replace a -> bool with true",
    ];
    let res = "--re=^crates/audio\\-core/src/block\\.rs:61:5: replace engine_block with \\(\\)$ \
               --re=^src/lib\\.rs:3:9: replace a \\-> bool with true$";
    let config = format!(
        "--cap-lints=true --gitignore=true --cargo-arg=--locked {} {}",
        crate::MUTANT_BUILD_CONFIG,
        crate::MUTANT_TEST_RUNNER
    );
    let timeouts = "--jobs 3 --timeout-multiplier 15 --minimum-test-timeout 60";
    let cases = [
        (
            crate::workspace_mutants_command(FAKE_JOBS, None),
            MUTANTS_OUT.to_owned(),
            format!(
                "cargo mutants --no-config --output=mutants.out/doc-tests {config} --workspace \
                 {timeouts} --cargo-test-arg=--doc {res} -- -- --include-ignored"
            ),
        ),
        (
            crate::workspace_mutants_command(FAKE_JOBS, crate::Shard::all().nth(1)),
            MUTANTS_OUT.to_owned(),
            format!(
                "cargo mutants --no-config --output=mutants.out/doc-tests {config} --workspace \
                 {timeouts} --cargo-test-arg=--doc {res} -- -- --include-ignored"
            ),
        ),
        (
            crate::changes_mutants_command(FAKE_JOBS),
            MUTANTS_OUT.to_owned(),
            format!(
                "cargo mutants --no-config --output=mutants.out/doc-tests {config} --workspace \
                 --in-diff target/xtask/changes.diff {timeouts} --cargo-test-arg=--doc {res} -- -- --include-ignored"
            ),
        ),
        (
            crate::gpu_mutants_command(),
            format!("{}/mutants.out", crate::GPU_MUTANTS_OUT),
            format!(
                "cargo mutants --no-config --output=mutants.out/gpu/mutants.out/doc-tests \
                 --cap-lints=true --cargo-arg=--locked {} {} --manifest-path \
                 crates/visual-gpu/Cargo.toml --in-place --cargo-test-arg=--doc {res} -- -- \
                 --include-ignored",
                crate::MUTANT_BUILD_CONFIG,
                crate::MUTANT_TEST_RUNNER
            ),
        ),
    ];
    for (first, report, expected) in cases {
        let pass = crate::steps::doc_test_pass_command(&first, &report, &missed);
        assert_eq!(
            pass.line(),
            expected,
            "cargo test (no nextest, no fail-fast, so the doc tests run and every test ends), \
             no shard, its own report below the first, the missed mutants alone, the rest of \
             {}",
            first.line()
        );
        assert_eq!(
            (pass.program, pass.dir),
            (first.program, first.dir),
            "as the first"
        );
    }
    let no_separator = Cmd::new("cargo", &["mutants", "--output=x"], Dir::Root);
    assert_eq!(
        crate::steps::doc_test_pass_command(&no_separator, "x/mutants.out", &[]).line(),
        "cargo mutants --output=x/mutants.out/doc-tests --cargo-test-arg=--doc --",
        "a run with no harness arguments gains an empty list of them"
    );
}

#[test]
fn every_rust_mutation_run_of_the_root_is_followed_by_its_doc_test_pass() {
    let jobs = FAKE_JOBS;
    let full = check_steps(Mode::Full { jobs });
    assert!(
        full.iter()
            .any(|step| step.action
                == Action::RustMutants(crate::workspace_mutants_command(jobs, None))),
        "the full check's"
    );
    for shard in crate::Shard::all() {
        assert!(
            crate::part_steps(crate::Part::Mutation(shard), jobs)
                .iter()
                .any(|step| step.action
                    == Action::RustMutants(crate::workspace_mutants_command(jobs, Some(shard)))),
            "shard {}'s",
            shard.index()
        );
    }
    let krate = crate::RUST_CRATES
        .into_iter()
        .find(|krate| krate.name == "audio-core")
        .expect("the audio library's core");
    let tested = crate::steps::tested_mutants_command(&["--package", "audio-core"], jobs);
    assert!(
        crate::mutants_steps(&crate::Target::Crate(krate), jobs)
            .iter()
            .any(|step| step.action == Action::RustMutants(tested.clone())),
        "`cargo xtask mutants`'s"
    );
    let runs_mutants = |step: &Step| matches!(&step.action, Action::Run(cmd) if cmd.args.first().is_some_and(|arg| arg == "mutants"));
    assert!(
        !full.iter().any(runs_mutants),
        "no cargo-mutants run without it"
    );
}

/// The runner's own program, run as cargo would run a test program under it on Linux: the
/// limit is in force in what it starts (the shell's `ulimit -v`, in KiB).
#[cfg(target_os = "linux")]
#[test]
fn the_mutant_runner_limits_what_it_starts_on_linux() {
    let runner: Vec<String> = crate::MUTANT_TEST_RUNNER
        .split_once(".runner=[")
        .and_then(|(_, list)| list.strip_suffix(']'))
        .expect("a runner list")
        .split(", ")
        .map(|item| item.trim_matches('"').to_owned())
        .collect();
    let (program, args) = runner.split_first().expect("a program");
    let output = std::process::Command::new(program)
        .args(args)
        .args(["sh", "-c", "ulimit -v"])
        .output()
        .expect("prlimit runs");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "4194304\n",
        "4 GiB in KiB, in force in the program started"
    );
}

#[test]
fn reports_every_failure_of_independent_gates() {
    use crate::steps::all_of;
    let failure = |summary: &str, details: &[&str]| Failure {
        summary: summary.to_owned(),
        details: details.iter().map(|&detail| detail.to_owned()).collect(),
        usage: false,
    };
    assert_eq!(all_of([Ok(()), Ok(())]), Ok(()), "none failed");
    assert_eq!(all_of(Vec::new()), Ok(()), "no gate");
    assert_eq!(
        all_of([
            Ok(()),
            Err(failure("a: 2 problems", &["one", "two"])),
            Ok(())
        ]),
        Err(failure("a: 2 problems", &["one", "two"])),
        "one failure, as it is"
    );
    assert_eq!(
        all_of([
            Err(failure("a: the one", &[])),
            Ok(()),
            Err(failure("b: 2 problems", &["one", "two"]))
        ]),
        Err(failure(
            "2 gates failed, listed above",
            &["a: the one", "one", "two", "b: 2 problems"]
        )),
        "two: each one's details, then its summary, in order"
    );
}

#[test]
fn runs_the_static_gates_of_the_full_check_alone() {
    let full = check_steps(Mode::Full {
        jobs: NonZeroUsize::MIN,
    });
    let gates = crate::gates_steps();
    assert_eq!(
        gates
            .iter()
            .map(|step| step.name.as_str())
            .collect::<Vec<_>>(),
        [
            "Tools and platform",
            "No mutant left in place in the GPU workspace",
            "Float methods of the pinned toolchain banned or reviewed as deterministic",
            "Rust documentation",
            "Rust documentation (GPU workspace)",
            "Files rustc loads: tracked, in their crate, those of tests alone named as test code",
            "TypeScript equivalent mutants reviewed",
            "Every bypass of coverage or mutation testing reviewed",
            "Unsafe code confined to the audited files",
            "Every root of both libraries denies its lints; every `no_std` crate built for every portability target or listed",
            "Cargo configuration: no build flag, a locked alias, the GPU workspace sharing the root's settings",
            "Test code only in tests/ and declared *_tests.rs files; a Rust test compares an error whole",
            "The tests of each crate read only its declared test data; each Rust command-line tool's tests read the shared vectors",
            "Tests turned off only where no test can hide, doc tests kept, each compile_fail \
             naming its error code",
            "Every function a review or a specification names exists, each new review naming its test; each reader of a file only a tool writes is named in the index",
            "Rust supply chain",
            "Rust supply chain (GPU workspace)",
            "Every direct dependency reviewed",
            "Bit-defining dependencies pinned to one exact version",
            "Every third-party build script and procedural macro reviewed; the crates both lock files share at one version",
            "Each library depends only on its own crates, its tests also on its test crates; \
             every crate only tests use is shared test code",
            "Every part under the license of its folder, no open part depending on an app",
            "Box arithmetic only through the math layer: libm in audio-core and audio-testkit \
             only, no raw float in box code, no iterator reduction outside the math layer",
            "Supply-chain configuration as xtask defines it",
            "The CPU reference built in the GPU workspace as at the root",
            "Packages and crates known to the checks",
            "Every part described in a README",
            "Every roadmap step cited exists on the board",
            "No prose counts a typed list",
            "The words of the glossary, not their synonyms",
            "No conflict marker left in a tracked text file",
            "Every tracked text file of a known kind",
            "No pattern a fix retired, in the code it was retired from",
            "Every hint a message gives names a command a user can run",
            "Every place that starts a process is listed; every copy of git's variables is xtask's",
            "Every reason of the registers of proof bypasses and equivalent mutants names what exists",
            "One version everywhere, changelog well formed, released sections as on main",
            "Commit messages, and a changelog entry for each change of behavior",
            "CI runs exactly the workflow xtask defines",
            "The parts of CI run every check of the full check once",
            "JavaScript dependencies",
            "JavaScript vulnerabilities",
            "JavaScript licenses",
            "Stryker configurations hide no mutant",
            "Each package's check runs exactly the scripts the checks time",
            "Rust unviable mutant reviews all current",
        ],
        "the preflight, the static gates in the order of the full check, then those only the \
         default check runs"
    );
    let changes = check_steps(Mode::Changes {
        jobs: NonZeroUsize::MIN,
    });
    let rest: Vec<&Step> = gates.iter().skip(1).collect();
    let only_changes = changes
        .iter()
        .filter(|step| step.action.is_static_gate() && !full.contains(step));
    assert_eq!(
        rest,
        full.iter()
            .filter(|step| step.action.is_static_gate())
            .chain(only_changes)
            .collect::<Vec<_>>(),
        "every static gate of either mode, each step as its check runs it"
    );
    assert_eq!(
        gates.first().map(|step| &step.action),
        Some(&Action::Preflight(crate::GpuDriver::Required)),
        "the preflight first"
    );
}

#[test]
fn checks_the_error_codes_of_the_doc_tests_of_the_workspace_crates_alone() {
    assert_eq!(
        crate::bootstrapped(&["a-b-c", "d"]),
        "a_b_c,d",
        "each name as rustc reads it, comma-separated"
    );
    assert_eq!(
        crate::doc_tests_command(crate::CargoWorkspace::Gpu).line(),
        "RUSTC_BOOTSTRAP=visual_gpu cargo test --manifest-path crates/visual-gpu/Cargo.toml \
         --workspace --doc --locked --target-dir target/doc-codes -- --include-ignored",
        "the GPU workspace's, alike"
    );
    assert_eq!(
        crate::doc_tests_command(crate::CargoWorkspace::Root).line(),
        "RUSTC_BOOTSTRAP=audio_core,audio_boxes,audio_patch,audio_hash,audio_host,visual_core,\
         visual_boxes,visual_hash,visual_testkit,visual_cli,visual_host,visual_wasm,\
         alloc_counter,fpenv,audio_battery,audio_testkit,audio_cli,audio_wasm,coordinator,web_page,xtask cargo test \
         --workspace --doc --locked --target-dir target/doc-codes -- --include-ignored",
        "the workspace's doc tests, a nightly compiler for its crates alone, in a target \
         directory of their own"
    );
    assert_eq!(
        crate::doc_codes_fixture_command().line(),
        "RUSTC_BOOTSTRAP=audio_core,audio_boxes,audio_patch,audio_hash,audio_host,visual_core,\
         visual_boxes,visual_hash,visual_testkit,visual_cli,visual_host,visual_wasm,\
         alloc_counter,fpenv,audio_battery,audio_testkit,audio_cli,audio_wasm,coordinator,web_page,xtask,doc_codes \
         cargo test --manifest-path crates/xtask/tests/fixtures/doc-codes/Cargo.toml --doc \
         --locked --target-dir target/doc-codes -- --include-ignored",
        "the fixture's, alike: the same list of crates, the fixture's after a comma, and the \
         same options"
    );
}

#[test]
fn proves_the_codes_are_checked_before_the_doc_tests_run() {
    let fixture = crate::doc_codes_fixture_command().line();
    let workspace = crate::doc_tests_command(crate::CargoWorkspace::Root).line();
    let gpu = crate::doc_tests_command(crate::CargoWorkspace::Gpu).line();
    let mut fake = passing();
    assert_eq!(
        crate::actions::doc_tests(&mut fake),
        Ok(()),
        "proven, then run"
    );
    assert_eq!(
        fake.ran,
        [fixture.clone(), workspace.clone(), gpu],
        "the fixture first, then each workspace's"
    );
    let mut unchecked = passing();
    let _passes = unchecked.files.remove(&format!("fails {fixture}"));
    let _passed = unchecked.files.insert(
        format!("stdout {fixture}"),
        "test src/lib.rs - (line 4) - compile fail ... ok\n".to_owned(),
    );
    assert_eq!(
        crate::actions::doc_tests(&mut unchecked),
        Err(Failure {
            summary: "the doc tests of crates/xtask/tests/fixtures/doc-codes passed: rustdoc \
                      did not check the error code of a compile_fail doc test"
                .to_owned(),
            details: vec!["test src/lib.rs - (line 4) - compile fail ... ok".to_owned()],
            usage: false,
        }),
        "a code unchecked, what rustdoc said kept"
    );
    assert_eq!(
        unchecked.ran,
        core::slice::from_ref(&fixture),
        "nothing more runs"
    );
    for stream in ["stdout", "stderr"] {
        let mut said = passing();
        let _moved = said.files.remove(&format!("stdout {fixture}"));
        let _said = said.files.insert(
            format!("{stream} {fixture}"),
            "Some expected error codes were not found: [\"E0599\"]\n".to_owned(),
        );
        assert_eq!(
            crate::actions::doc_tests(&mut said),
            Ok(()),
            "said on {stream}"
        );
    }
    let mut other = passing();
    let _cause = other.files.insert(
        format!("stdout {fixture}"),
        "error[E0425]: cannot find value `missing` in this scope\n".to_owned(),
    );
    let _said = other.files.insert(
        format!("stderr {fixture}"),
        "error: doctest failed\nerror: could not compile `doc-codes`\n".to_owned(),
    );
    assert_eq!(
        crate::actions::doc_tests(&mut other),
        Err(Failure {
            summary: format!(
                "the doc tests of crates/xtask/tests/fixtures/doc-codes failed for another \
                 reason than its error code: `{fixture}` exited with code 3: error: doctest \
                 failed"
            ),
            details: [
                "error[E0425]: cannot find value `missing` in this scope",
                "error: doctest failed",
                "error: could not compile `doc-codes`",
            ]
            .map(str::to_owned)
            .to_vec(),
            usage: false,
        }),
        "another failure, rustdoc's cause kept: its output, then the error output"
    );
    let mut failing = passing();
    failing.fail = Some(workspace.clone());
    assert_eq!(
        crate::actions::doc_tests(&mut failing),
        Err(Failure::from(format!("`{workspace}` exited with code 3"))),
        "a doc test of the workspace fails"
    );
}

#[test]
fn rustdoc_checks_the_error_code_of_the_fixture_under_the_variable() {
    use crate::Runner as _;
    // The real tools: the fixture's doc test names E0599 and fails with E0425. Run as the
    // step runs it, it fails saying so; without the variable, the code is not checked.
    let target = scratch("doc-codes");
    let target_dir = target.path().to_str().expect("a UTF-8 path").to_owned();
    let mut runner = crate::tests::system(crate::tests::fixture("doc-codes"));
    let in_scratch = |cmd: Cmd| {
        let mut args: Vec<String> = cmd
            .args
            .iter()
            .filter(|arg| !arg.starts_with("--manifest-path") && !arg.ends_with("Cargo.toml"))
            .cloned()
            .collect();
        let at = args
            .iter()
            .position(|arg| arg == crate::DOC_TESTS_TARGET)
            .expect("its target directory");
        if let Some(arg) = args.get_mut(at) {
            arg.clone_from(&target_dir);
        }
        Cmd { args, ..cmd }
    };
    let checked = runner.run_for_stdout(&in_scratch(crate::doc_codes_fixture_command()));
    assert!(
        checked.ended.is_err() && checked.stdout.contains(crate::UNCHECKED_CODE),
        "refused for its code: {}{}",
        checked.stdout,
        checked.stderr
    );
    let mut bare = in_scratch(crate::doc_codes_fixture_command());
    bare.env.clear();
    let unchecked = runner.run_for_stdout(&bare);
    assert!(
        unchecked.ended.is_ok(),
        "without the variable, the stable compiler passes it: {}{}",
        unchecked.stdout,
        unchecked.stderr
    );
}

#[test]
fn adds_the_gpu_workspaces_unviable_mutants_to_the_roots_report() {
    let command = crate::gpu_mutants_command();
    assert!(
        command
            .line()
            .starts_with("cargo mutants --no-config --output=mutants.out/gpu --cap-lints=true")
            && command.line().ends_with(
                "--manifest-path crates/visual-gpu/Cargo.toml --in-place -- -- --include-ignored"
            ),
        "every mutant, reported apart, tested in place: {}",
        command.line()
    );
    assert!(
        !command
            .args
            .iter()
            .any(|arg| arg.starts_with("--gitignore")),
        "no copy's option, which cargo-mutants refuses with --in-place: {}",
        command.line()
    );
    assert!(
        command
            .args
            .iter()
            .any(|arg| arg == crate::MUTANT_TEST_RUNNER),
        "under the memory limit of every cargo-mutants run"
    );
    let gpu: Vec<Step> = check_steps(Mode::Full {
        jobs: NonZeroUsize::MIN,
    })
    .into_iter()
    .filter(|step| matches!(step.action, Action::GpuMutants(_)))
    .collect();
    let report = format!("{}/mutants.out/unviable.txt", crate::GPU_MUTANTS_OUT);
    let mut fake = passing();
    let _root = fake.files.insert(
        UNVIABLE_REPORT.to_owned(),
        "crates/a/src/b.rs: r\n".to_owned(),
    );
    let _gpu = fake
        .files
        .insert(report.clone(), "src/lib.rs: s\nsrc/lib.rs: t\n".to_owned());
    assert_eq!(run_steps(&gpu, &mut fake, &mut Vec::new()), Ok(()));
    assert_eq!(
        fake.files.get(UNVIABLE_REPORT).map(String::as_str),
        Some(
            "crates/a/src/b.rs: r\ncrates/visual-gpu/src/lib.rs: s\ncrates/visual-gpu/src/lib.rs: t\n"
        ),
        "the root's, then the GPU workspace's, from the root"
    );
    let name = "Rust mutation testing (GPU workspace)";
    for (broken, expected) in [
        (
            crate::gpu_mutants_command().line(),
            format!(
                "{name}: cannot run `{}`: not found",
                crate::gpu_mutants_command().line()
            ),
        ),
        (
            format!("write {UNVIABLE_REPORT}"),
            format!("{name}: cannot write {UNVIABLE_REPORT}: read-only"),
        ),
    ] {
        let mut broken_fake = passing();
        broken_fake.broken = Some(broken.clone());
        assert_eq!(
            run_steps(&gpu, &mut broken_fake, &mut Vec::new()),
            failed(expected),
            "{broken}"
        );
    }
    let mut unreported = passing();
    let _removed = unreported.files.remove(&report);
    assert_eq!(
        run_steps(&gpu, &mut unreported, &mut Vec::new()),
        failed(format!("{name}: cannot read {report}: missing")),
        "a complete run always writes its report"
    );
}

#[test]
fn tells_the_steps_that_run_code_of_the_gpu_workspace() {
    let cargo = |args: &[&str]| Action::Run(Cmd::new("cargo", args, Dir::Root));
    let gpu = ["--manifest-path", crate::GPU_MANIFEST];
    for subcommand in ["test", "run", "nextest", "llvm-cov", "mutants", "miri"] {
        assert!(
            cargo(&[subcommand, gpu[0], gpu[1]]).runs_gpu_code(),
            "{subcommand} runs what it builds"
        );
    }
    assert!(
        cargo(&["+nightly-2026-11-01", "llvm-cov", gpu[0], gpu[1]]).runs_gpu_code(),
        "a toolchain named before the subcommand"
    );
    for (action, why) in [
        (cargo(&["clippy", gpu[0], gpu[1]]), "lints run nothing"),
        (cargo(&["fmt", gpu[0], gpu[1]]), "formatting runs nothing"),
        (cargo(&["test"]), "the root's tests"),
        (cargo(&["test", gpu[0], "Cargo.toml"]), "another manifest"),
        (cargo(&["test", gpu[1]]), "the manifest's path alone"),
        (cargo(&["+nightly-2026-11-01"]), "no subcommand"),
        (
            Action::Run(Cmd::new("pnpm", &["test", gpu[0], gpu[1]], Dir::Root)),
            "not cargo",
        ),
        (
            Action::ToolGate(Cmd::new("cargo", &["doc", gpu[0], gpu[1]], Dir::Root)),
            "a static gate's tool classed by its command: documentation runs nothing",
        ),
    ] {
        assert!(!action.runs_gpu_code(), "{why}");
    }
    // Finding F17-6 of the fourth deep review of visual block 2 (#34): every static gate's
    // tool was classed as running no GPU code, whatever its command ran.
    assert!(
        Action::ToolGate(Cmd::new("cargo", &["test", gpu[0], gpu[1]], Dir::Root)).runs_gpu_code(),
        "a static gate's tool that runs the GPU workspace's tests"
    );
    let coverages: Vec<&str> = check_steps(Mode::Full { jobs: FAKE_JOBS })
        .into_iter()
        .filter_map(|step| {
            let runs = step.action.runs_gpu_code();
            if let Action::Coverage(coverage) = step.action {
                runs.then_some(coverage.krate.name)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        coverages,
        ["visual-gpu", "visual-gpu"],
        "the coverage of the GPU workspace's one crate alone, its library and its binary"
    );
    for action in [
        Action::GpuMutants(Extent::Complete),
        Action::DocTests,
        Action::RustMutantsOfChanges { jobs: FAKE_JOBS },
    ] {
        assert!(action.runs_gpu_code(), "{action:?}");
    }
    for action in [
        Action::RustMutants(Cmd::new("cargo", &["mutants"], Dir::Root)),
        Action::Preflight(crate::GpuDriver::Required),
        Action::CoverageLocality,
        Action::FusedBuild,
    ] {
        assert!(!action.runs_gpu_code(), "{action:?}");
    }
}

#[test]
fn runs_in_each_native_part_what_ci_cannot_prove() {
    let gpu = "cargo --manifest-path crates/visual-gpu/Cargo.toml";
    let expected: [(crate::NativePart, &str, &[&str], Vec<String>); 3] = [
        (
            crate::NativePart::Audio,
            "audio",
            &["fpenv", "audio-cli", "audio-core"],
            vec![
                "Root: cargo test --package fpenv --package audio-cli --package audio-core --locked -- --include-ignored"
                    .to_owned(),
                format!(
                    "conformance {}",
                    crate::conformance_command().line()
                ),
            ],
        ),
        (
            crate::NativePart::Visual,
            "visual",
            &["visual-cli"],
            vec![
                "Root: cargo run --release --package visual-cli --locked --quiet -- check conformance/visual"
                    .to_owned(),
            ],
        ),
        (
            crate::NativePart::Gpu,
            "GPU",
            &["visual-gpu"],
            vec![
                format!("Root: {} --workspace --locked -- --include-ignored", gpu.replacen("cargo", "cargo test", 1)),
                format!("Root: {} --locked --quiet -- check", gpu.replacen("cargo", "cargo run", 1)),
                "fresh mutants.out".to_owned(),
                format!("gpu mutants (Partial) {}", crate::gpu_mutants_command().line()),
                "unviable gate (Partial)".to_owned(),
            ],
        ),
    ];
    assert_eq!(
        expected.iter().map(|(part, ..)| *part).collect::<Vec<_>>(),
        crate::NativePart::ALL,
        "every part, in order"
    );
    for (part, name, crates, lines) in expected {
        assert_eq!(part.name(), name, "{part:?}");
        assert_eq!(part.crates(), crates, "{part:?}");
        assert!(
            crates
                .iter()
                .all(|named| crate::RUST_CRATES.iter().any(|krate| krate.name == *named)),
            "{part:?}: crates xtask knows"
        );
        assert_eq!(command_lines(&part.steps()), lines, "{part:?}");
    }
}

#[test]
fn types_each_native_step_once_as_the_checks_type_it() {
    // Each step of a native part is a step of the default or the full check, the same name
    // and the same action, but the two only the native check runs: the tests of the native
    // shell, its floating-point mode and the audio core on this machine's architecture, and
    // every mutant of the GPU workspace as a partial run, the root's not run.
    let jobs = FAKE_JOBS;
    let checks: Vec<Step> = [Mode::Full { jobs }, Mode::Changes { jobs }]
        .into_iter()
        .flat_map(check_steps)
        .collect();
    let own: Vec<String> = crate::NativePart::ALL
        .into_iter()
        .flat_map(crate::NativePart::steps)
        .filter(|step| !checks.contains(step))
        .map(|step| step.name)
        .collect();
    assert_eq!(
        own,
        [
            "Rust tests of the native shell, its floating-point mode and the audio core, on this \
             machine's architecture",
            "Rust mutation testing (GPU workspace)",
        ],
        "the steps of the native check alone"
    );
}

#[test]
fn frames_the_native_parts_with_the_tree_recorded_then_unchanged() {
    for parts in [
        &[][..],
        &[crate::NativePart::Audio, crate::NativePart::Gpu],
        &crate::NativePart::ALL,
    ] {
        let inner: Vec<String> = parts
            .iter()
            .flat_map(|part| part.steps())
            .map(|step| step.name)
            .collect();
        let actions: Vec<Action> = crate::native_steps(parts)
            .into_iter()
            .map(|step| step.action)
            .collect();
        assert_eq!(
            crate::native_steps(parts)
                .into_iter()
                .map(|step| step.name)
                .collect::<Vec<_>>(),
            [
                vec!["Working tree recorded".to_owned()],
                inner,
                vec!["Working tree unchanged by the checks".to_owned()]
            ]
            .concat(),
            "{parts:?}: the tree recorded, each part, then the tree unchanged"
        );
        assert_eq!(
            (actions.first(), actions.last()),
            (Some(&Action::TreeRecorded), Some(&Action::TreeUnchanged)),
            "{parts:?}"
        );
    }
}
