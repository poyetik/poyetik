//! Tests of the command line of xtask: its commands, usage and targets.

use core::num::NonZeroUsize;
use std::ffi::OsString;
use std::io;

use crate::tests::{
    Broken, CHANGES_MODE, Fake, HERMETIC, MAIN_BASE, MERGE_BASE, PART_NAMES, PUSH_STATUS,
    added_package_steps, command_lines, elapsed, failed, hermetic_git, one_mutant, passing,
    run_cli, scratch, system, visual_steps,
};
use crate::{
    Action, BestEffort, Failure, PACKAGES, Step, SystemRunner, UNVIABLE_REPORT, UNVIABLE_REVIEWED,
    USAGE, bless_benchmarks_steps, bless_step, check_steps, cli, run_steps,
};

#[test]
fn reads_the_record_real_git_holds_at_a_commit() {
    let dir = scratch("record-at");
    std::fs::create_dir_all(dir.join("conformance/algorithms.json.d")).expect("a directory");
    std::fs::write(dir.join("conformance/algorithms.json.d/x"), "x\n").expect("a near name");
    std::fs::write(dir.join("conformance/algorithms.jsonx"), "y\n").expect("a near name");
    let _init = hermetic_git(&dir, &["init", "--quiet", "--initial-branch=main"]);
    let _add = hermetic_git(&dir, &["add", "."]);
    let _commit = hermetic_git(&dir, &["commit", "--quiet", "-m", "without"]);
    let without = hermetic_git(&dir, &["rev-parse", "HEAD"]);
    std::fs::write(
        dir.join("conformance/algorithms.json"),
        "{\n  \"a\": 1\n}\n",
    )
    .expect("a record");
    let _add_record = hermetic_git(&dir, &["add", "."]);
    let _record = hermetic_git(&dir, &["commit", "--quiet", "-m", "with"]);
    let with = hermetic_git(&dir, &["rev-parse", "HEAD"]);
    let mut runner = system(dir.path());
    assert_eq!(
        crate::file_at(&mut runner, without.trim(), crate::CONFORMANCE_RECORD)
            .map_err(|failure| failure.summary),
        Ok(None),
        "none at a commit without it, whatever names begin like it"
    );
    assert_eq!(
        crate::file_at(&mut runner, with.trim(), crate::CONFORMANCE_RECORD)
            .map_err(|failure| failure.summary),
        Ok(Some("{\n  \"a\": 1\n}\n".to_owned())),
        "its bytes at a commit with it"
    );
    assert!(
        crate::file_at(
            &mut runner,
            "0000000000000000000000000000000000000000",
            crate::CONFORMANCE_RECORD,
        )
        .is_err_and(|failure| failure.summary.starts_with(
            "cannot read conformance/algorithms.json at the merge base \
                 0000000000000000000000000000000000000000: "
        )),
        "a commit git does not have"
    );
}

#[test]
fn reads_every_kind_of_mutation_target() {
    use crate::{Target, mutation_target};
    let krate = |name: &str| {
        *crate::RUST_CRATES
            .iter()
            .find(|krate| krate.name == name)
            .expect("a crate")
    };
    let package = |dir: &str| {
        *PACKAGES
            .iter()
            .find(|package| package.dir == dir)
            .expect("a package")
    };
    assert_eq!(
        mutation_target("audio-core"),
        Ok(Target::Crate(krate("audio-core"))),
        "a crate, by its name"
    );
    assert_eq!(
        mutation_target("--changed"),
        Ok(Target::ChangedLines),
        "the lines changed"
    );
    assert_eq!(Target::ChangedLines.file(), None, "no file");
    assert_eq!(
        mutation_target("crates/xtask/src/lib.rs"),
        Ok(Target::RustFile {
            krate: krate("xtask"),
            path: "crates/xtask/src/lib.rs".to_owned()
        }),
        "a Rust file, in its crate"
    );
    assert_eq!(
        mutation_target("bindings/audio-wasm/src/ffi.rs"),
        Ok(Target::RustFile {
            krate: krate("audio-wasm"),
            path: "bindings/audio-wasm/src/ffi.rs".to_owned()
        }),
        "a Rust file of the crate that shares a package's directory"
    );
    assert_eq!(
        mutation_target("bindings/audio-wasm"),
        Ok(Target::Package(package("bindings/audio-wasm"))),
        "a package, by its directory"
    );
    for (path, dir, source) in [
        ("apps/roadmap/src/board.ts", "apps/roadmap", "src/board.ts"),
        (
            "apps/playground/src/app.tsx",
            "apps/playground",
            "src/app.tsx",
        ),
        (
            "bindings/audio-wasm/ts/worklet/x.mts",
            "bindings/audio-wasm",
            "ts/worklet/x.mts",
        ),
        ("tooling/oracle/src/a.cts", "tooling/oracle", "src/a.cts"),
        ("tooling/config/eslint.js", "tooling/config", "eslint.js"),
        ("tooling/config/a.jsx", "tooling/config", "a.jsx"),
        ("tooling/config/a.mjs", "tooling/config", "a.mjs"),
        ("tooling/config/a.cjs", "tooling/config", "a.cjs"),
    ] {
        assert_eq!(
            mutation_target(path),
            Ok(Target::TypeScriptFile {
                package: package(dir),
                source: source.to_owned()
            }),
            "{path}: a source, in its package"
        );
    }
}

#[test]
fn refuses_what_is_no_mutation_target() {
    for (text, reason) in [
        (
            "apps/roadmap/test/board.test.ts",
            "\"apps/roadmap/test/board.test.ts\" is test code: name the source it tests",
        ),
        (
            "apps/roadmap/src/board.test.ts",
            "\"apps/roadmap/src/board.test.ts\" is test code: name the source it tests",
        ),
        (
            "crates/xtask/../audio-core/src/lib.rs",
            "\"crates/xtask/../audio-core/src/lib.rs\" is not a plain path from the root (no `.`, `..` or empty part)",
        ),
        (
            "./crates/xtask/src/lib.rs",
            "\"./crates/xtask/src/lib.rs\" is not a plain path from the root (no `.`, `..` or empty part)",
        ),
        (
            "crates/xtask//src/lib.rs",
            "\"crates/xtask//src/lib.rs\" is not a plain path from the root (no `.`, `..` or empty part)",
        ),
        (
            "apps/roadmap/",
            "\"apps/roadmap/\" is not a plain path from the root (no `.`, `..` or empty part)",
        ),
        (
            "crates/xtask",
            "\"crates/xtask\" is no crate, Rust file of a crate, TypeScript package or source of one, nor `--changed`",
        ),
        (
            "crates/other/src/lib.rs",
            "\"crates/other/src/lib.rs\" is no crate, Rust file of a crate, TypeScript package or source of one, nor `--changed`",
        ),
        (
            "crates/xtaskx/src/lib.rs",
            "\"crates/xtaskx/src/lib.rs\" is no crate, Rust file of a crate, TypeScript package or source of one, nor `--changed`",
        ),
        (
            "apps/roadmap/src/page.astro",
            "\"apps/roadmap/src/page.astro\" is no crate, Rust file of a crate, TypeScript package or source of one, nor `--changed`",
        ),
        (
            "apps/roadmap/README.md",
            "\"apps/roadmap/README.md\" is no crate, Rust file of a crate, TypeScript package or source of one, nor `--changed`",
        ),
        (
            "apps/roadmap/src/lib.rs",
            "\"apps/roadmap/src/lib.rs\" is no crate, Rust file of a crate, TypeScript package or source of one, nor `--changed`",
        ),
        (
            "crates/audio-core/src/a.ts",
            "\"crates/audio-core/src/a.ts\" is no crate, Rust file of a crate, TypeScript package or source of one, nor `--changed`",
        ),
        (
            "crates/xtask/src/lib_tests.rs",
            "\"crates/xtask/src/lib_tests.rs\" is test code: name the source it tests",
        ),
        (
            "crates/xtask/tests/binary.rs",
            "\"crates/xtask/tests/binary.rs\" is test code: name the source it tests",
        ),
        (
            "crates/xtask/tests/fixtures/coverage-locality/src/lib.rs",
            "\"crates/xtask/tests/fixtures/coverage-locality/src/lib.rs\" is test code: name the source it tests",
        ),
        (
            "crates/xtask/src/lib.RS",
            "\"crates/xtask/src/lib.RS\" is no crate, Rust file of a crate, TypeScript package or source of one, nor `--changed`",
        ),
        (
            "\u{1b}[2J",
            "\"\\u{1b}[2J\" is no crate, Rust file of a crate, TypeScript package or source of one, nor `--changed`",
        ),
    ] {
        assert_eq!(
            crate::mutation_target(text),
            Err(reason.to_owned()),
            "{text}"
        );
    }
}

#[test]
fn mutates_every_mutant_of_one_target() {
    use crate::{mutants_steps, mutation_target};
    let jobs = NonZeroUsize::new(3).expect("three is not zero");
    let lines = |text: &str| {
        command_lines(&mutants_steps(
            &mutation_target(text).expect("a target"),
            jobs,
        ))
    };
    assert_eq!(
        lines("audio-core"),
        vec![
            "fresh mutants.out",
            "Root: cargo mutants --no-config --output=. --cap-lints=true --gitignore=true --cargo-arg=--locked --cargo-arg=--config=profile.test.debug=false --cargo-arg=--config=target.'cfg(target_os = \"linux\")'.runner=[\"prlimit\", \"--as=4294967296\"] --package audio-core --jobs 3 --timeout-multiplier 15 --minimum-test-timeout 60 -- -- --include-ignored",
            "unviable gate (Partial)",
        ],
        "a crate"
    );
    assert_eq!(
        lines("crates/xtask/src/lib.rs"),
        vec![
            "fresh mutants.out",
            "Root: cargo mutants --no-config --output=. --cap-lints=true --gitignore=true --cargo-arg=--locked --cargo-arg=--config=profile.test.debug=false --cargo-arg=--config=target.'cfg(target_os = \"linux\")'.runner=[\"prlimit\", \"--as=4294967296\"] --package xtask --file crates/xtask/src/lib.rs --jobs 3 --timeout-multiplier 15 --minimum-test-timeout 60 -- -- --include-ignored",
            "unviable gate (Partial)",
        ],
        "a Rust file"
    );
    assert_eq!(
        lines("apps/roadmap"),
        vec![
            "fresh apps/roadmap/reports/mutation",
            "Package(\"apps/roadmap\"): pnpm exec stryker run stryker.config.json --incremental --incrementalFile reports/stryker-incremental.json --force",
            "typescript mutation gate of apps/roadmap (Complete)",
        ],
        "a package, every mutant, its incremental file rewritten"
    );
    assert_eq!(
        lines("apps/roadmap/src/board.ts"),
        vec![
            "fresh apps/roadmap/reports/mutation",
            "Package(\"apps/roadmap\"): pnpm exec stryker run stryker.config.json --mutate src/board.ts",
            "typescript mutation gate of apps/roadmap (Partial)",
        ],
        "a source: its incremental file untouched"
    );
    let names = |text: &str| -> Vec<String> {
        mutants_steps(&mutation_target(text).expect("a target"), jobs)
            .into_iter()
            .map(|step| step.name)
            .collect()
    };
    assert_eq!(
        names("crates/xtask/src/lib.rs"),
        vec![
            "Fresh Rust mutation report",
            "Rust mutation testing of crates/xtask/src/lib.rs",
            "Rust unviable mutants reviewed"
        ],
        "named"
    );
    assert_eq!(
        names("audio-core")[1],
        "Rust mutation testing of audio-core",
        "named"
    );
    assert_eq!(
        names("apps/roadmap/src/board.ts"),
        vec![
            "Fresh mutation report of apps/roadmap",
            "Mutation testing of apps/roadmap/src/board.ts",
            "TypeScript mutants of apps/roadmap/src/board.ts killed"
        ],
        "named"
    );
    assert_eq!(
        names("apps/roadmap")[1],
        "Mutation testing of apps/roadmap",
        "named"
    );
}

#[test]
fn cli_mutates_one_target() {
    let mut out = Vec::new();
    let mut fake = passing();
    assert_eq!(
        run_cli(
            &["mutants".to_owned(), "audio-core".to_owned()],
            &mut fake,
            &mut out,
            &mut Vec::new()
        ),
        0,
        "a crate"
    );
    assert_eq!(
        fake.ran,
        vec![
            "fresh mutants.out",
            "cargo mutants --no-config --output=. --cap-lints=true --gitignore=true --cargo-arg=--locked --cargo-arg=--config=profile.test.debug=false --cargo-arg=--config=target.'cfg(target_os = \"linux\")'.runner=[\"prlimit\", \"--as=4294967296\"] --package audio-core --jobs 3 --timeout-multiplier 15 --minimum-test-timeout 60 -- -- --include-ignored",
            "read mutants.out/unviable.txt",
            "read mutants-unviable.txt",
        ],
        "in half the threads of the machine"
    );
    assert!(
        String::from_utf8(out)
            .expect("UTF-8")
            .ends_with("  done in 1.5 s\n✔ all 3 checks passed in 4.5 s\n"),
        "said"
    );
    let mut source = passing();
    assert_eq!(
        run_cli(
            &["mutants".to_owned(), "apps/roadmap/src/a.ts".to_owned()],
            &mut source,
            &mut Vec::new(),
            &mut Vec::new()
        ),
        0,
        "a source that exists"
    );
    assert_eq!(
        source.ran.first().map(String::as_str),
        Some("read apps/roadmap/src/a.ts"),
        "read first"
    );
    assert!(
        source
            .ran
            .contains(&"pnpm exec stryker run stryker.config.json --mutate src/a.ts".to_owned())
            && source
                .ran
                .contains(&"read apps/roadmap/reports/mutation/mutation.json".to_owned()),
        "mutated, its report gated: {:?}",
        source.ran
    );
    let mut survivor = passing();
    let _report = survivor.files.insert(
        "apps/roadmap/reports/mutation/mutation.json".to_owned(),
        one_mutant("Survived", "M", 4),
    );
    let mut err = Vec::new();
    assert_eq!(
        run_cli(
            &["mutants".to_owned(), "apps/roadmap".to_owned()],
            &mut survivor,
            &mut Vec::new(),
            &mut err
        ),
        1,
        "a survivor fails"
    );
    assert_eq!(
        String::from_utf8(err).expect("UTF-8"),
        "error: TypeScript mutants of apps/roadmap killed: apps/roadmap: mutants not killed: apps/roadmap/x.ts:4 M Survived\n",
        "named"
    );
    let mut unlisted = passing();
    let _sources = unlisted.broken.replace("list *.ts *.tsx".to_owned());
    assert_eq!(
        run_cli(
            &["mutants".to_owned(), "apps/roadmap".to_owned()],
            &mut unlisted,
            &mut Vec::new(),
            &mut Vec::new()
        ),
        1,
        "the disable comments must be read"
    );
}

#[test]
fn refuses_a_mutation_target_in_one_line() {
    for (args, reason) in [
        (vec!["mutants"], "`mutants` takes one target"),
        (vec!["mutants", "a", "b"], "`mutants` takes one target"),
        (
            vec!["mutants", "crates/xtask/src/gone.rs"],
            "cannot read crates/xtask/src/gone.rs: missing",
        ),
        (
            vec!["mutants", "apps/roadmap/src/gone.ts"],
            "cannot read apps/roadmap/src/gone.ts: missing",
        ),
    ] {
        let args: Vec<String> = args.into_iter().map(str::to_owned).collect();
        let mut usage = Vec::new();
        let mut fake = passing();
        assert_eq!(
            run_cli(&args, &mut fake, &mut Vec::new(), &mut usage),
            2,
            "{args:?}"
        );
        assert_eq!(
            String::from_utf8(usage).expect("UTF-8"),
            format!("error: {reason} (see: cargo xtask help)\n"),
            "{args:?}"
        );
        assert!(
            !fake.ran.iter().any(|line| !line.starts_with("read ")),
            "{args:?}: nothing runs"
        );
    }
}

#[test]
fn cli_runs_check_and_reports_errors() {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    assert_eq!(
        run_cli(&["check".to_owned()], &mut passing(), &mut out, &mut err),
        0,
        "check passes"
    );
    assert!(err.is_empty(), "no error output");

    let mut failing = passing();
    failing.fail = Some("cargo deny --locked check".to_owned());
    let mut err2 = Vec::new();
    assert_eq!(
        run_cli(
            &["check".to_owned()],
            &mut failing,
            &mut Vec::new(),
            &mut err2
        ),
        1,
        "check fails"
    );
    assert_eq!(
        String::from_utf8(err2).expect("UTF-8"),
        "error: Rust supply chain: `cargo deny --locked check` exited with code 3\n",
        "the command and its exit code on stderr"
    );

    let mut killed = passing();
    killed.killed = Some("cargo deny --locked check".to_owned());
    let mut err3 = Vec::new();
    assert_eq!(
        run_cli(
            &["check".to_owned()],
            &mut killed,
            &mut Vec::new(),
            &mut err3
        ),
        1,
        "a stopped command fails the check"
    );
    assert_eq!(
        String::from_utf8(err3).expect("UTF-8"),
        "error: Rust supply chain: `cargo deny --locked check` was stopped by signal 9 (SIGKILL)\n",
        "explained"
    );

    let mut help = Vec::new();
    assert_eq!(
        run_cli(
            &["help".to_owned()],
            &mut passing(),
            &mut help,
            &mut Vec::new()
        ),
        0,
        "help succeeds"
    );
    assert_eq!(help, format!("{USAGE}\n").into_bytes(), "help on stdout");
    assert_eq!(
        run_cli(
            &["help".to_owned()],
            &mut passing(),
            &mut Broken,
            &mut Vec::new()
        ),
        1,
        "broken stdout"
    );

    assert_eq!(
        run_cli(&[], &mut passing(), &mut Vec::new(), &mut Broken),
        2,
        "broken stderr still tells"
    );
    assert_eq!(
        run_cli(
            &["commit-msg".to_owned(), "missing".to_owned()],
            &mut passing(),
            &mut Vec::new(),
            &mut Broken
        ),
        1,
        "broken stderr still fails"
    );
}

#[test]
fn ignores_a_closed_reader_but_not_other_output_errors() {
    use io::Write as _;
    let mut closed = BestEffort(Pipe(io::ErrorKind::BrokenPipe));
    assert_eq!(
        closed.write(b"abc").map_err(|error| error.kind()),
        Ok(3),
        "write"
    );
    assert_eq!(
        closed.flush().map_err(|error| error.kind()),
        Ok(()),
        "flush"
    );
    let mut full = BestEffort(Pipe(io::ErrorKind::StorageFull));
    assert_eq!(
        full.write(b"abc").map_err(|error| error.kind()),
        Err(io::ErrorKind::StorageFull),
        "write"
    );
    assert_eq!(
        full.flush().map_err(|error| error.kind()),
        Err(io::ErrorKind::StorageFull),
        "flush"
    );
}

/// What the preflight asks and reads in a passing fake: the ten tools, `.node-version`
/// and the toolchain file.
const PREFLIGHT_RAN: [&str; 12] = [
    "capture git --version",
    "capture cargo --version",
    "capture cargo +nightly-2026-11-01 --version",
    "capture cargo +nightly-2026-11-01 miri --version",
    "capture cargo llvm-cov --version",
    "capture cargo mutants --version",
    "capture cargo deny --version",
    "capture node --version",
    "capture pnpm --version",
    "capture rustc --version",
    "read .node-version",
    "read rust-toolchain.toml",
];

#[test]
fn bless_benchmarks_counts_and_rewrites_the_baseline() {
    let steps = bless_benchmarks_steps();
    assert_eq!(
        steps
            .iter()
            .map(|step| step.name.as_str())
            .collect::<Vec<_>>(),
        ["Build the engine module", "Bless the benchmark baseline"],
        "names"
    );
    assert_eq!(
        command_lines(&steps),
        vec![
            "Package(\"bindings/audio-wasm\"): pnpm run build:wasm",
            "Package(\"bindings/audio-wasm\"): node scripts/benchmarks.ts bless"
        ],
        "builds the module, then runs the benchmark command's bless, which no package script runs"
    );
    let mut fake = passing();
    let mut out = Vec::new();
    assert_eq!(
        run_cli(
            &["bless-benchmarks".to_owned()],
            &mut fake,
            &mut out,
            &mut Vec::new()
        ),
        0,
        "bless-benchmarks succeeds"
    );
    assert_eq!(
        fake.ran,
        [
            PREFLIGHT_RAN.as_slice(),
            &["pnpm run build:wasm", "node scripts/benchmarks.ts bless"]
        ]
        .concat(),
        "the preflight, then only the build and the benchmark bless run"
    );
    assert_eq!(
        String::from_utf8(out).expect("utf-8"),
        "▶ [1/3] Tools and platform\n  done in 1.5 s\n▶ [2/3] Build the engine module\n  done in \
         1.5 s\n▶ [3/3] Bless the benchmark baseline\n  done in 1.5 s\n✔ all 3 checks passed in \
         4.5 s\n",
        "progress"
    );
}

/// A passing fake: its spent base resolves ([`MAIN_BASE`]).
fn blessing() -> Fake {
    passing()
}

/// How the fake lists the record of the conformance references at its merge base.
const RECORD_AT_BASE: &str = "capture git ls-tree --name-only \
                              fed4560000000000000000000000000000000000 -- \
                              conformance/algorithms.json";

/// How the fake shows the record of the conformance references at its merge base.
const SHOW_RECORD: &str =
    "capture git show fed4560000000000000000000000000000000000:conformance/algorithms.json";

/// The command `cargo xtask bless` runs, without options.
const BLESS_RAN: &str = "cargo run --package audio-cli --locked --quiet -- bless conformance \
                         --spent target/xtask/spent-algorithms.json";

#[test]
fn bless_reports_the_failure_of_the_bless_it_runs() {
    let mut fake = Fake {
        fail: Some(BLESS_RAN.to_owned()),
        ..blessing()
    };
    drop(fake.files.insert(RECORD_AT_BASE.to_owned(), String::new()));
    let mut err = Vec::new();
    assert_eq!(
        run_cli(&["bless".to_owned()], &mut fake, &mut Vec::new(), &mut err),
        1,
        "a failing bless fails the command"
    );
    assert_eq!(
        fake.ran.last().map(String::as_str),
        Some(BLESS_RAN),
        "the bless ran last"
    );
    assert_eq!(
        String::from_utf8(err).expect("utf-8"),
        format!("error: Bless conformance references: `{BLESS_RAN}` exited with code 3\n"),
        "the bless's own failure, once"
    );
}

#[test]
fn passes_its_options_on_and_tells_a_refused_one_as_a_usage_error() {
    let options = [
        "--frm-nothing",
        "--case-changed",
        "a",
        "--case-changed",
        "a",
    ];
    let ran = format!("{BLESS_RAN} {}", options.join(" "));
    let args: Vec<String> = core::iter::once(&"bless")
        .chain(&options)
        .map(|word| (*word).to_owned())
        .collect();
    let mut fake = Fake {
        refuse: Some(ran.clone()),
        ..blessing()
    };
    drop(fake.files.insert(RECORD_AT_BASE.to_owned(), String::new()));
    drop(
        fake.files
            .insert(format!("stdout {ran}"), "OPTION a\n".to_owned()),
    );
    drop(fake.files.insert(
        format!("stderr {ran}"),
        "error: unknown option \"--frm-nothing\" (see: audio-cli help)\n".to_owned(),
    ));
    let (mut out, mut err) = (Vec::new(), Vec::new());
    assert_eq!(
        run_cli(&args, &mut fake, &mut out, &mut err),
        2,
        "audio-cli's usage error is xtask's"
    );
    assert_eq!(
        fake.ran.last(),
        Some(&ran),
        "every option passed on unread, in its order, a repeated one included"
    );
    assert_eq!(
        String::from_utf8(err).expect("utf-8"),
        "error: Bless conformance references: `audio-cli bless` refused the options given to \
         `cargo xtask bless`: unknown option \"--frm-nothing\" (see: cargo xtask help)\n",
        "one line, with the child's reason, pointing to the usage"
    );
    assert!(
        String::from_utf8(out).expect("utf-8").contains(
            "  OPTION a\n  error: unknown option \"--frm-nothing\" (see: audio-cli help)\n  \
             failed after"
        ),
        "what it said on stdout, then on stderr, replayed in that order"
    );
    // Another failure of the same command is a failure, exit 1, without the pointer.
    let mut failing = Fake {
        fail: Some(ran.clone()),
        ..blessing()
    };
    drop(
        failing
            .files
            .insert(RECORD_AT_BASE.to_owned(), String::new()),
    );
    let mut failed_err = Vec::new();
    assert_eq!(
        run_cli(&args, &mut failing, &mut Vec::new(), &mut failed_err),
        1,
        "a failure"
    );
    assert_eq!(
        String::from_utf8(failed_err).expect("utf-8"),
        format!("error: Bless conformance references: `{ran}` exited with code 3\n"),
        "said as a failure"
    );
    // A usage error of a command among several steps is still xtask's usage error: the
    // flag survives the step's summary.
    let mut refused = Fake {
        refuse: Some(BLESS_RAN.to_owned()),
        ..blessing()
    };
    drop(
        refused
            .files
            .insert(RECORD_AT_BASE.to_owned(), String::new()),
    );
    let mut refused_out = Vec::new();
    assert_eq!(
        run_steps(&[bless_step(&[])], &mut refused, &mut refused_out),
        Err(Failure {
            summary: "Bless conformance references: `audio-cli bless` refused the options \
                      given to `cargo xtask bless`"
                .to_owned(),
            details: Vec::new(),
            usage: true,
        }),
        "a usage error, through the step"
    );
}

#[test]
fn refuses_to_be_given_the_record_spent() {
    let mut fake = blessing();
    let mut err = Vec::new();
    assert_eq!(
        run_cli(
            &["bless", "--spent", "x"].map(str::to_owned),
            &mut fake,
            &mut Vec::new(),
            &mut err
        ),
        2,
        "a usage error"
    );
    assert_eq!(
        String::from_utf8(err).expect("utf-8"),
        "error: `bless` gives `--spent` itself: the spent record (see: cargo xtask help)\n",
        "said"
    );
    assert!(fake.ran.is_empty(), "before anything runs: {:?}", fake.ran);
}

#[test]
fn replays_the_result_lines_of_the_bless_on_stdout() {
    let results = "CHANGED sine-1000 (run bless)\nBITS sine-1000 binary32\n";
    let mut fake = blessing();
    drop(fake.files.insert(RECORD_AT_BASE.to_owned(), String::new()));
    drop(
        fake.files
            .insert(format!("stdout {BLESS_RAN}"), results.to_owned()),
    );
    let (mut out, mut err) = (Vec::new(), Vec::new());
    assert_eq!(
        run_cli(&["bless".to_owned()], &mut fake, &mut out, &mut err),
        0,
        "bless succeeds"
    );
    let said = String::from_utf8(out).expect("utf-8");
    assert!(
        said.contains("  CHANGED sine-1000 (run bless)\n  BITS sine-1000 binary32\n  done in"),
        "each result line on stdout, as the step's: {said}"
    );
    assert!(err.is_empty(), "nothing on stderr");
    let mut refused = blessing();
    drop(
        refused
            .files
            .insert(RECORD_AT_BASE.to_owned(), String::new()),
    );
    drop(
        refused
            .files
            .insert(format!("stdout {BLESS_RAN}"), "MISMATCH a\n".to_owned()),
    );
    refused.fail = Some(BLESS_RAN.to_owned());
    let mut refused_out = Vec::new();
    assert_eq!(
        run_cli(
            &["bless".to_owned()],
            &mut refused,
            &mut refused_out,
            &mut Vec::new()
        ),
        1,
        "bless fails"
    );
    assert!(
        String::from_utf8(refused_out)
            .expect("utf-8")
            .contains("  MISMATCH a\n  failed after"),
        "what it refuses, on stdout too"
    );
}

/// `cargo xtask bless` through real processes: real git in a scratch repository whose main
/// holds no record, and a stand-in for `cargo` that refuses an option as `audio-cli` does,
/// with a result line on stdout and its error line on stderr. The step fails as a usage
/// error, its last line carrying the child's reason, after the child's lines in their order.
/// That cargo passes on the exit code of `audio-cli` is its documented behavior, review
/// only.
#[cfg(unix)]
#[test]
fn tells_a_refused_option_of_a_real_bless_as_a_usage_error_with_its_reason() {
    use std::os::unix::fs::PermissionsExt as _;
    let root = scratch("bless-usage");
    let _init = hermetic_git(&root, &["init", "-q", "-b", "main"]);
    std::fs::write(root.join("a.txt"), "a\n").expect("file written");
    let _add = hermetic_git(&root, &["add", "a.txt"]);
    let _commit = hermetic_git(&root, &["commit", "-q", "-m", "docs: start"]);
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).expect("bin created");
    let stub = bin.join("cargo");
    std::fs::write(
        &stub,
        "#!/bin/sh\necho 'OPTION   --bogus'\necho 'error: unknown option \"--bogus\" (see: audio-cli \
         help)' >&2\nexit 2\n",
    )
    .expect("stub written");
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755))
        .expect("stub executable");
    let path: &'static str = Box::leak(
        format!(
            "{}:{}",
            bin.display(),
            std::env::var("PATH").expect("a PATH")
        )
        .into_boxed_str(),
    );
    let env: &'static [(&'static str, &'static str)] = Box::leak(
        HERMETIC
            .into_iter()
            .chain([("PATH", path)])
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    );
    let mut runner = SystemRunner {
        root: root.to_path_buf(),
        env,
    };
    let mut out = Vec::new();
    let failure = run_steps(
        &[bless_step(&["--bogus".to_owned()])],
        &mut runner,
        &mut out,
    )
    .expect_err("refused");
    assert_eq!(
        (failure.usage, failure.summary),
        (
            true,
            "Bless conformance references: `audio-cli bless` refused the options given to \
             `cargo xtask bless`: unknown option \"--bogus\""
                .to_owned()
        ),
        "a usage error, with the reason the child gave"
    );
    let shown = String::from_utf8(out).expect("UTF-8");
    assert!(
        shown.contains(
            "  OPTION   --bogus\n  error: unknown option \"--bogus\" (see: audio-cli help)\n"
        ),
        "the child's result line, then its error line: {shown}"
    );
    assert_eq!(
        std::fs::read_to_string(root.join(crate::SPENT_RECORD))
            .ok()
            .as_deref(),
        Some("null\n"),
        "the record at the merge base, none"
    );
}

#[test]
fn bless_renders_and_rewrites_the_references() {
    let step = bless_step(&[]);
    assert_eq!(step.name, "Bless conformance references", "name");
    assert_eq!(
        command_lines(&[step]),
        [format!("bless {BLESS_RAN}")],
        "blesses with the record at the merge base"
    );
    let mut fake = blessing();
    drop(fake.files.insert(
        RECORD_AT_BASE.to_owned(),
        "conformance/algorithms.json\n".to_owned(),
    ));
    drop(
        fake.files
            .insert(SHOW_RECORD.to_owned(), "{\"spent\": 1}\n".to_owned()),
    );
    let mut out = Vec::new();
    assert_eq!(
        run_cli(&["bless".to_owned()], &mut fake, &mut out, &mut Vec::new()),
        0,
        "bless succeeds"
    );
    assert_eq!(
        fake.ran,
        [
            PREFLIGHT_RAN.as_slice(),
            &[
                "capture git merge-base HEAD origin/main",
                RECORD_AT_BASE,
                SHOW_RECORD,
                "write target/xtask/spent-algorithms.json",
                BLESS_RAN,
            ]
        ]
        .concat(),
        "the preflight, the record at the merge base, then bless with it"
    );
    assert_eq!(
        fake.files.get(crate::SPENT_RECORD).map(String::as_str),
        Some("{\"spent\": 1}\n"),
        "the record at the merge base, as git gives it"
    );
    assert_eq!(
        String::from_utf8(out).expect("utf-8"),
        "▶ [1/2] Tools and platform\n  done in 1.5 s\n▶ [2/2] Bless conformance references\n  \
         spent: its versions at fed4560000000000000000000000000000000000 (merge base with \
         origin/main)\n  done in 1.5 s\n✔ all 2 checks passed in 3.0 s\n",
        "progress"
    );
    let mut named = blessing();
    drop(named.files.insert(RECORD_AT_BASE.to_owned(), String::new()));
    let mut named_out = Vec::new();
    assert_eq!(
        run_cli(
            &[
                "bless",
                "--case-changed",
                "a",
                "--from-nothing",
                "--case-changed",
                "b c",
                "--battery-changed"
            ]
            .map(str::to_owned),
            &mut named,
            &mut named_out,
            &mut Vec::new()
        ),
        0,
        "bless with every option succeeds"
    );
    assert_eq!(
        named.ran.last().map(String::as_str),
        Some(
            format!(
                "{BLESS_RAN} --case-changed a --from-nothing --case-changed b c --battery-changed"
            )
            .as_str()
        ),
        "every option passed on unread, in its order"
    );
    assert!(
        !named.ran.contains(&SHOW_RECORD.to_owned()),
        "nothing to show"
    );
    assert_eq!(
        named.files.get(crate::SPENT_RECORD).map(String::as_str),
        Some("null\n"),
        "no record at the merge base: nothing spent"
    );
    assert!(
        String::from_utf8(named_out)
            .expect("utf-8")
            .contains("  spent: none at fed4560000000000000000000000000000000000"),
        "said"
    );
}

#[test]
fn blesses_only_with_the_record_at_the_merge_base() {
    let mut cases: Vec<(Fake, &str)> = Vec::new();
    // The upstream of the branch resolves (MERGE_BASE), neither origin/main nor main: the
    // versions pushed on the branch are not spent.
    let mut baseless = passing();
    let _main = baseless.files.remove(&format!("capture {}", MAIN_BASE.0));
    cases.push((
        baseless,
        "error: Bless conformance references: no merge base with origin/main or main to read \
         the algorithm versions spent from (fetch or create one of them)\n",
    ));
    let mut unlisted = blessing();
    unlisted.broken = Some(RECORD_AT_BASE.trim_start_matches("capture ").to_owned());
    cases.push((
        unlisted,
        "error: Bless conformance references: cannot read conformance/algorithms.json at the \
         merge base fed4560000000000000000000000000000000000: ",
    ));
    let mut unshown = blessing();
    drop(unshown.files.insert(
        RECORD_AT_BASE.to_owned(),
        "conformance/algorithms.json\n".to_owned(),
    ));
    cases.push((
        unshown,
        "error: Bless conformance references: cannot read conformance/algorithms.json at the \
         merge base fed4560000000000000000000000000000000000: ",
    ));
    let mut unwritable = blessing();
    drop(
        unwritable
            .files
            .insert(RECORD_AT_BASE.to_owned(), String::new()),
    );
    unwritable.broken = Some("write target/xtask/spent-algorithms.json".to_owned());
    cases.push((
        unwritable,
        "error: Bless conformance references: cannot write target/xtask/spent-algorithms.json: \
         read-only",
    ));
    for (mut fake, said) in cases {
        let mut err = Vec::new();
        assert_eq!(
            run_cli(&["bless".to_owned()], &mut fake, &mut Vec::new(), &mut err),
            1,
            "{said}"
        );
        assert!(
            String::from_utf8(err).expect("utf-8").starts_with(said),
            "{said}"
        );
        assert!(
            !fake.ran.contains(&BLESS_RAN.to_owned()),
            "{said}: nothing blessed"
        );
        assert!(
            !fake.ran.contains(&format!("capture {}", MERGE_BASE.0)),
            "{said}: the upstream never asked"
        );
    }
}

/// The command of the conformance step.
const CONFORMANCE_RAN: &str = "cargo run --package audio-cli --locked --quiet -- check \
                               conformance --spent target/xtask/spent-algorithms.json";

#[test]
fn checks_the_conformance_against_the_record_on_main() {
    let step = Step {
        name: "Conformance (native)".to_owned(),
        action: Action::Conformance,
    };
    let mut fake = passing();
    drop(fake.files.insert(
        RECORD_AT_BASE.to_owned(),
        "conformance/algorithms.json\n".to_owned(),
    ));
    drop(
        fake.files
            .insert(SHOW_RECORD.to_owned(), "{\"spent\": 1}\n".to_owned()),
    );
    let mut out = Vec::new();
    assert_eq!(
        run_steps(core::slice::from_ref(&step), &mut fake, &mut out),
        Ok(()),
        "checked"
    );
    assert_eq!(
        fake.ran,
        [
            "capture git merge-base HEAD origin/main",
            RECORD_AT_BASE,
            SHOW_RECORD,
            "write target/xtask/spent-algorithms.json",
            CONFORMANCE_RAN,
        ],
        "the spent record written, then the check given it"
    );
    assert_eq!(
        fake.files.get(crate::SPENT_RECORD).map(String::as_str),
        Some("{\"spent\": 1}\n"),
        "as git gives it"
    );
    assert!(
        String::from_utf8(out).expect("utf-8").contains(
            "  spent: its versions at fed4560000000000000000000000000000000000 (merge base with \
             origin/main)\n"
        ),
        "said"
    );
    let mut failing = Fake {
        fail: Some(CONFORMANCE_RAN.to_owned()),
        ..passing()
    };
    assert_eq!(
        run_steps(core::slice::from_ref(&step), &mut failing, &mut Vec::new()),
        failed(format!(
            "Conformance (native): `{CONFORMANCE_RAN}` exited with code 3"
        )),
        "the check's failure"
    );
    let mut baseless = passing();
    let _main = baseless.files.remove(&format!("capture {}", MAIN_BASE.0));
    assert!(
        run_steps(core::slice::from_ref(&step), &mut baseless, &mut Vec::new()).is_err_and(
            |failure| failure.summary
                == "Conformance (native): no merge base with origin/main or main to read the \
                    algorithm versions spent from (fetch or create one of them)"
        ),
        "no spent record to compare with: the step fails"
    );
    assert!(
        !baseless.ran.contains(&CONFORMANCE_RAN.to_owned()),
        "nothing checked without it"
    );
}

#[test]
fn blesses_only_where_the_checks_could_run() {
    // A bless writes what every later check compares with: it starts with the preflight of
    // the check, so that a build drifted by the environment (a profile, a wrapper, an
    // outside Cargo configuration) is never blessed.
    for (task, script) in [
        ("bless", BLESS_RAN),
        ("bless-benchmarks", "node scripts/benchmarks.ts bless"),
    ] {
        let mut fake = passing();
        fake.environment.variables = vec!["CARGO_PROFILE_RELEASE_OPT_LEVEL".to_owned()];
        let mut err = Vec::new();
        assert_eq!(
            run_cli(&[task.to_owned()], &mut fake, &mut Vec::new(), &mut err),
            1,
            "{task}: a drifting variable fails"
        );
        assert_eq!(
            String::from_utf8(err).expect("utf-8"),
            "error: Tools and platform: the checks cannot run here: \
             CARGO_PROFILE_RELEASE_OPT_LEVEL is set, which changes what cargo builds or how it \
             lints: unset it\n",
            "{task}: the preflight's reason"
        );
        assert!(
            !fake.ran.contains(&script.to_owned()),
            "{task}: nothing blessed"
        );
        let mut configured = passing();
        configured.environment.cargo_configs = vec!["/home/x/.cargo/config.toml".to_owned()];
        assert_eq!(
            run_cli(
                &[task.to_owned()],
                &mut configured,
                &mut Vec::new(),
                &mut Vec::new()
            ),
            1,
            "{task}: an outside Cargo configuration fails"
        );
        assert!(
            !configured.ran.contains(&script.to_owned()),
            "{task}: nothing blessed"
        );
    }
}

#[test]
fn lists_the_checks_in_order() {
    let mut out = Vec::new();
    assert_eq!(
        run_cli(
            &["list".to_owned()],
            &mut passing(),
            &mut out,
            &mut Vec::new()
        ),
        0,
        "listed"
    );
    let text = String::from_utf8(out).expect("UTF-8");
    assert!(
        text.starts_with(
            " 1. Tools and platform\n 2. Working tree recorded\n 3. JavaScript dependencies\n"
        ),
        "{text}"
    );
    assert_eq!(
        text.lines().count(),
        check_steps(CHANGES_MODE).len(),
        "every check"
    );
    assert!(
        text.contains("Rust mutation testing of the changes\n"),
        "of the default check: {text}"
    );
    let mut full = Vec::new();
    let mut fake = passing();
    assert_eq!(
        run_cli(
            &["list".to_owned(), "--full".to_owned()],
            &mut fake,
            &mut full,
            &mut Vec::new()
        ),
        0,
        "listed"
    );
    let full = String::from_utf8(full).expect("UTF-8");
    assert!(
        full.contains(". Rust mutation testing\n") && !full.contains("of the changes"),
        "of the full check: {full}"
    );
    assert!(
        full.starts_with(
            " 1. Tools and platform\n 2. Working tree recorded\n 3. Rust formatting\n"
        ),
        "in the order of the full check: {full}"
    );
    assert!(fake.ran.is_empty(), "listing runs nothing");
    assert_eq!(
        run_cli(
            &["list".to_owned()],
            &mut passing(),
            &mut Broken,
            &mut Vec::new()
        ),
        1,
        "broken stdout"
    );
}

#[test]
fn runs_the_full_check_before_pushing_a_tag() {
    let head = "1111111111111111111111111111111111111111";
    let mut tagged = passing();
    for (command, answer) in [
        ("git rev-parse HEAD".to_owned(), format!("{head}\n")),
        (
            "git status --porcelain --untracked-files=all --ignore-submodules=none".to_owned(),
            String::new(),
        ),
        (
            format!("git rev-parse --verify {head}^{{commit}}"),
            format!("{head}\n"),
        ),
    ] {
        let _answer = tagged.files.insert(format!("capture {command}"), answer);
    }
    let release = [
        "pre-push".to_owned(),
        head.to_owned(),
        "refs/heads/main".to_owned(),
        head.to_owned(),
        "refs/tags/v1.0.0".to_owned(),
    ];
    assert_eq!(
        run_cli(&release, &mut tagged, &mut Vec::new(), &mut Vec::new()),
        0,
        "a release"
    );
    assert!(
        tagged.ran.contains(
            &"cargo mutants --no-config --output=. --cap-lints=true --gitignore=true --cargo-arg=--locked --cargo-arg=--config=profile.test.debug=false --cargo-arg=--config=target.'cfg(target_os = \"linux\")'.runner=[\"prlimit\", \"--as=4294967296\"] --workspace --jobs 3 --timeout-multiplier 15 --minimum-test-timeout 60 -- -- --include-ignored".to_owned()
        ) && !tagged.ran.contains(&format!("capture {}", MERGE_BASE.0)),
        "a pushed tag runs every mutant: {:?}",
        tagged.ran
    );
}

#[test]
fn checks_a_push_then_runs_every_check() {
    let head = "1111111111111111111111111111111111111111";
    let mut fake = passing();
    let first = fake
        .files
        .insert("capture git rev-parse HEAD".to_owned(), format!("{head}\n"));
    assert!(first.is_none(), "fresh fake");
    assert_eq!(
        fake.files.get(&format!("capture {}", PUSH_STATUS.0)),
        Some(&String::new()),
        "a clean tree"
    );
    let peeled = fake.files.insert(
        format!("capture git rev-parse --verify {head}^{{commit}}"),
        format!("{head}\n"),
    );
    assert!(peeled.is_none(), "fresh fake");
    let mut out = Vec::new();
    let args = [
        "pre-push".to_owned(),
        head.to_owned(),
        "refs/heads/work".to_owned(),
    ];
    assert_eq!(
        run_cli(&args, &mut fake, &mut out, &mut Vec::new()),
        0,
        "clean push"
    );
    let checks = 115 + visual_steps() + added_package_steps();
    assert!(
        String::from_utf8(out).expect("UTF-8").ends_with(&format!(
            "✔ all {checks} checks passed in {}\n",
            elapsed(checks)
        )),
        "then every check"
    );
    assert!(
        fake.ran.contains(&format!("capture {}", MERGE_BASE.0))
            && fake.ran.contains(
                &"cargo mutants --no-config --output=. --cap-lints=true --gitignore=true --cargo-arg=--locked --cargo-arg=--config=profile.test.debug=false --cargo-arg=--config=target.'cfg(target_os = \"linux\")'.runner=[\"prlimit\", \"--as=4294967296\"] --workspace --in-diff target/xtask/changes.diff --jobs 3 --timeout-multiplier 15 --minimum-test-timeout 60 -- -- --include-ignored"
                    .to_owned()
            ),
        "the default check: the hook mutates the changes"
    );
    let mut dirty = passing();
    let third = dirty
        .files
        .insert("capture git rev-parse HEAD".to_owned(), format!("{head}\n"));
    let fourth = dirty.files.insert(
        "capture git status --porcelain --untracked-files=all --ignore-submodules=none".to_owned(),
        " M a.rs\n".to_owned(),
    );
    let fifth = dirty.files.insert(
        format!("capture git rev-parse --verify {head}^{{commit}}"),
        format!("{head}\n"),
    );
    assert!(
        third.is_none() && fourth.is_some() && fifth.is_none(),
        "fresh fake, but its clean status"
    );
    let mut err = Vec::new();
    assert_eq!(
        run_cli(&args, &mut dirty, &mut Vec::new(), &mut err),
        1,
        "dirty tree"
    );
    assert!(
        dirty.ran.iter().all(|line| !line.contains("cargo fmt")),
        "no check runs"
    );
    assert_eq!(
        String::from_utf8(err).expect("UTF-8"),
        "error: the push cannot be checked: the working tree has changes not committed, which the check would include: M a.rs\n",
        "says what to commit"
    );
    let mut blind = passing();
    let mut blind_err = Vec::new();
    assert_eq!(
        run_cli(&args, &mut blind, &mut Vec::new(), &mut blind_err),
        1,
        "no git"
    );
    assert_eq!(
        String::from_utf8(blind_err).expect("UTF-8"),
        "error: cannot read the work tree: `git rev-parse HEAD` exited with code 1: failed\n",
        "says what it could not read"
    );
}

#[test]
fn prints_the_details_of_a_failure_on_stdout_and_one_line_on_stderr() {
    let mut fake = passing();
    let unreviewed = fake.files.insert(
        UNVIABLE_REPORT.to_owned(),
        "crates/a/src/c.rs:1:1: replace g -> U with x\n".to_owned(),
    );
    assert!(unreviewed.is_some(), "report replaced");
    let (mut out, mut err) = (Vec::new(), Vec::new());
    assert_eq!(
        run_cli(
            &["check".to_owned(), "--full".to_owned()],
            &mut fake,
            &mut out,
            &mut err
        ),
        1,
        "the check fails"
    );
    assert!(
        fake.ran.contains(
            &"cargo mutants --no-config --output=. --cap-lints=true --gitignore=true --cargo-arg=--locked --cargo-arg=--config=profile.test.debug=false --cargo-arg=--config=target.'cfg(target_os = \"linux\")'.runner=[\"prlimit\", \"--as=4294967296\"] --workspace --jobs 3 --timeout-multiplier 15 --minimum-test-timeout 60 -- -- --include-ignored".to_owned()
        ),
        "every mutant, in half the threads of the machine"
    );
    let checks = 113 + visual_steps() + added_package_steps();
    let took = elapsed(checks);
    assert_eq!(
        String::from_utf8(err).expect("UTF-8"),
        format!("error: 1 of {checks} checks failed, listed above\n"),
        "one line on stderr, counting the failed steps"
    );
    assert!(
        String::from_utf8(out).expect("UTF-8").ends_with(&format!(
            "▶ [{checks}/{checks}] Working tree unchanged by the checks\n  done in 1.5 s\n\
             ✘ 1 of {checks} checks failed in {took}\n\
             not reviewed: crates/a/src/c.rs: replace g -> U with x\n\
             stale review: crates/a/src/b.rs: replace f -> T with Default::default()\n\
             Rust unviable mutants reviewed: unviable mutants must be reviewed in \
             {UNVIABLE_REVIEWED}: 2 problems, listed above\n"
        )),
        "every step run, then the problems and the failed step on stdout"
    );
}

#[test]
fn aligns_the_usage_in_the_order_of_its_first_line() {
    let mut lines = USAGE.lines();
    let first = lines.next().expect("a first line");
    let listed: Vec<&str> = first
        .trim_start_matches("usage: cargo xtask <")
        .trim_end_matches('>')
        .split('|')
        .filter_map(|form| form.split_whitespace().next())
        .collect();
    let mut described = Vec::new();
    let mut columns = Vec::new();
    for line in lines {
        let body = line.trim_start();
        let indent = line.len() - body.len();
        if indent == 2 {
            described.push(body.split_whitespace().next().expect("a command"));
            let (_, description) = body
                .split_once("  ")
                .expect("a description after two spaces");
            columns.push(line.len() - description.trim_start().len());
        } else {
            columns.push(indent);
        }
    }
    assert_eq!(listed, described, "the rows follow the first line");
    assert!(
        columns.windows(2).all(|pair| pair[0] == pair[1]),
        "every description starts at the same column: {columns:?}"
    );
}

#[test]
fn names_in_the_usage_the_bases_xtask_reads_against() {
    let words = USAGE.split_whitespace().collect::<Vec<_>>().join(" ");
    let named = |base: &str| {
        if base == "@{upstream}" {
            "the upstream".to_owned()
        } else {
            base.to_owned()
        }
    };
    let (last, first) = crate::MUTATION_BASES
        .split_last()
        .expect("at least one base");
    let changes = format!(
        "the merge base with {} or {};",
        first
            .iter()
            .map(|base| named(base))
            .collect::<Vec<_>>()
            .join(", "),
        named(last)
    );
    let spent = format!(
        "at the merge base with {})",
        crate::MAIN_BASES.map(named).join(", else ")
    );
    for phrase in [changes, spent] {
        assert!(words.contains(&phrase), "{phrase}");
    }
}

/// The usage error of `check` with other arguments than it takes.
const CHECK_USAGE: &str = "`check` takes no argument but `--full`, then `--part <part>`";

/// The usage error of `list` with other arguments than it takes.
const LIST_USAGE: &str = "`list` takes no argument but `--full`, then `--parts`";

#[test]
fn refuses_usage_errors_in_one_line() {
    for (args, reason) in [
        (vec![], "no command given"),
        (vec!["nope"], "unknown command \"nope\""),
        (vec!["nope", "x"], "unknown command \"nope\""),
        (vec!["check", "x"], CHECK_USAGE),
        (vec!["check", "--full", "x"], CHECK_USAGE),
        (vec!["check", "--part", "rust"], CHECK_USAGE),
        (vec!["check", "--full", "--part"], CHECK_USAGE),
        (vec!["check", "x", "--part", "rust"], CHECK_USAGE),
        (vec!["check", "--full", "x", "rust"], CHECK_USAGE),
        (vec!["check", "--full", "--part", "rust", "x"], CHECK_USAGE),
        (
            vec!["check", "--full", "--part", "mutation-8"],
            "unknown part \"mutation-8\" (`cargo xtask list --full --parts` lists them)",
        ),
        (
            vec!["check", "--full", "--part", "tooling/config"],
            "unknown part \"tooling/config\" (`cargo xtask list --full --parts` lists them)",
        ),
        (vec!["list", "x"], LIST_USAGE),
        (vec!["list", "--full", "--full"], LIST_USAGE),
        (vec!["list", "--parts"], LIST_USAGE),
        (vec!["list", "x", "--parts"], LIST_USAGE),
        (vec!["list", "--full", "--parts", "x"], LIST_USAGE),
        (
            vec!["bless", "--case-changed", "a", "--spent", "b"],
            "`bless` gives `--spent` itself: the spent record",
        ),
        (
            vec!["bless-benchmarks", "x"],
            "`bless-benchmarks` takes no argument",
        ),
        (vec!["help", "x"], "`help` takes no argument"),
        (vec!["commit-msg"], "`commit-msg` takes one file"),
        (
            vec!["pre-push", "abc"],
            "`pre-push` takes pairs of a commit and the remote ref it updates",
        ),
        (
            vec!["pre-push", "abc", "refs/heads/a", "def"],
            "`pre-push` takes pairs of a commit and the remote ref it updates",
        ),
        (vec!["commit-msg", "a", "b"], "`commit-msg` takes one file"),
    ] {
        let args: Vec<String> = args.into_iter().map(str::to_owned).collect();
        let (mut stdout, mut usage) = (Vec::new(), Vec::new());
        let mut fake = passing();
        assert_eq!(
            run_cli(&args, &mut fake, &mut stdout, &mut usage),
            2,
            "{args:?} is a usage error"
        );
        assert_eq!(
            String::from_utf8(usage).expect("UTF-8"),
            format!("error: {reason} (see: cargo xtask help)\n"),
            "{args:?}: one line on stderr, the reason and where to look"
        );
        assert!(stdout.is_empty(), "{args:?}: nothing on stdout");
        assert!(fake.ran.is_empty(), "{args:?}: nothing runs");
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt as _;
        let mut usage = Vec::new();
        assert_eq!(
            cli(
                &[OsString::from_vec(b"che\xffck".to_vec())],
                &mut passing(),
                &mut Vec::new(),
                &mut usage
            ),
            2,
            "an argument that is not UTF-8 is a usage error, not a panic"
        );
        assert_eq!(
            String::from_utf8(usage).expect("UTF-8"),
            "error: argument is not valid UTF-8: che\u{fffd}ck (see: cargo xtask help)\n",
            "named as well as it can be"
        );
    }
}

#[test]
fn lists_and_runs_the_parts_of_the_full_check() {
    let args =
        |words: &[&str]| -> Vec<String> { words.iter().map(|word| (*word).to_owned()).collect() };
    let mut listed = Vec::new();
    assert_eq!(
        run_cli(
            &args(&["list", "--full", "--parts"]),
            &mut passing(),
            &mut listed,
            &mut Vec::new()
        ),
        0,
        "listed"
    );
    assert_eq!(
        String::from_utf8(listed).expect("UTF-8"),
        format!("{}\n", PART_NAMES.join("\n")),
        "one part per line"
    );
    let mut fake = passing();
    let mut progress = Vec::new();
    assert_eq!(
        run_cli(
            &args(&["check", "--full", "--part", "miri"]),
            &mut fake,
            &mut progress,
            &mut Vec::new()
        ),
        0,
        "the part passes"
    );
    assert!(
        String::from_utf8(progress).expect("UTF-8").ends_with(
            "▶ [4/4] Working tree unchanged by the checks\n  done in 1.5 s\n✔ all 4 checks \
                 passed in 6.0 s\n"
        ),
        "its own steps only"
    );
    let mut failing = passing();
    failing.fail = Some(crate::miri_command().line());
    let mut err = Vec::new();
    let mut shown = Vec::new();
    let code = run_cli(
        &args(&["check", "--full", "--part", "miri"]),
        &mut failing,
        &mut shown,
        &mut err,
    );
    let text = String::from_utf8(shown).expect("UTF-8");
    assert_eq!(code, 1, "the part fails");
    assert!(
        text.contains("▶ [4/4] Working tree unchanged by the checks\n"),
        "every step runs whatever fails, as in the full check: {text}"
    );
    assert_eq!(
        String::from_utf8(err).expect("UTF-8"),
        "error: 1 of 4 checks failed, listed above\n",
        "every failure counted"
    );
}

/// An output whose reader is gone, or that fails another way.
struct Pipe(io::ErrorKind);

impl io::Write for Pipe {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        Err(io::Error::from(self.0))
    }
    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::from(self.0))
    }
}

#[test]
fn cli_mutates_the_lines_changed() {
    let mut changed = passing();
    assert_eq!(
        run_cli(
            &["mutants".to_owned(), "--changed".to_owned()],
            &mut changed,
            &mut Vec::new(),
            &mut Vec::new()
        ),
        0,
        "the lines changed"
    );
    assert_eq!(
        changed.ran.first().map(String::as_str),
        Some("fresh mutants.out"),
        "its report emptied first"
    );
    assert!(
        changed.ran.contains(&"cargo mutants --no-config --output=. --cap-lints=true --gitignore=true --cargo-arg=--locked --cargo-arg=--config=profile.test.debug=false --cargo-arg=--config=target.'cfg(target_os = \"linux\")'.runner=[\"prlimit\", \"--as=4294967296\"] --workspace --in-diff target/xtask/changes.diff --jobs 3 --timeout-multiplier 15 --minimum-test-timeout 60 -- -- --include-ignored".to_owned())
            && changed.ran.ends_with(&[
                "read mutants.out/unviable.txt".to_owned(),
                "read mutants-unviable.txt".to_owned()
            ]),
        "mutated in half the threads, its unviable mutants gated: {:?}",
        changed.ran
    );
}
