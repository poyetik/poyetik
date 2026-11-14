//! Tests of the coverage gates, Rust and TypeScript, and the state of the tree they measure.

use crate::tests::{
    CHANGES_MODE, TREE_HEAD, TREE_STATUS, TS_COVERED, failed, found, full_report, hermetic_git,
    passing, report, scratch, system,
};
use crate::{
    Action, Coverage, Measured, RustCrate, Step, check_steps, coverage_gate, run_steps,
    ts_coverage_gate,
};

/// The library measurement of a crate named `name` in `crates/a`.
const CRATE_A: Coverage = Coverage {
    krate: RustCrate {
        name: "a",
        dir: "crates/a",
        binary: true,
        test_data: &[],
        roles: &[],
    },
    measured: Measured::Library,
};

/// The binary measurement of [`CRATE_A`].
const CRATE_A_MAIN: Coverage = Coverage {
    measured: Measured::Binary,
    ..CRATE_A
};

#[test]
fn accepts_full_coverage() {
    assert_eq!(
        coverage_gate(
            &report(&[
                ("crates/a/src/lib.rs", (10, 10), (3, 3), (4, 4)),
                ("crates/a/src/b.rs", (1, 1), (1, 1), (0, 0)),
            ]),
            CRATE_A
        ),
        Ok(()),
        "every file of the crate at 100%, a file without branches included"
    );
    assert_eq!(
        coverage_gate(
            &report(&[
                ("crates/a/src/lib.rs", (10, 1), (3, 1), (4, 1)),
                ("crates/a/src/main.rs", (5, 5), (1, 1), (0, 0)),
            ]),
            CRATE_A_MAIN
        ),
        Ok(()),
        "a binary: only its main.rs is read"
    );
}

#[test]
fn names_the_files_that_are_not_fully_covered() {
    assert_eq!(
        coverage_gate(
            &report(&[
                ("crates/a/src/b.rs", (10, 9), (3, 3), (4, 4)),
                ("crates/a/src/c.rs", (10, 10), (3, 2), (4, 3)),
            ]),
            CRATE_A
        ),
        failed(format!(
            "a is not covered 100% by its own unit tests: 2 problems, listed above\n\
             crates/a/src/b.rs: lines 9/10\n\
             crates/a/src/c.rs: functions 2/3, branches 3/4\n\
             the lines: cargo {} llvm-cov --manifest-path crates/a/Cargo.toml --lib --branch \
             --show-missing-lines -- --include-ignored",
            crate::NIGHTLY_TOOLCHAIN
        )),
        "every file and every incomplete kind, with the command showing the lines"
    );
    assert_eq!(
        coverage_gate(
            &report(&[("crates/a/src/main.rs", (5, 4), (1, 1), (0, 0))]),
            CRATE_A_MAIN
        ),
        failed(format!(
            "the a binary is not covered 100% by its tests: crates/a/src/main.rs: lines 4/5\n\
             the lines: cargo {} llvm-cov --manifest-path crates/a/Cargo.toml --branch \
             --show-missing-lines -- --include-ignored",
            crate::NIGHTLY_TOOLCHAIN
        )),
        "a binary, measured by every test of its crate"
    );
}

#[test]
fn coverage_gate_refuses_files_of_another_crate() {
    assert_eq!(
        coverage_gate(
            &report(&[
                ("crates/a/src/lib.rs", (10, 10), (3, 3), (4, 4)),
                ("crates/ab/src/lib.rs", (10, 10), (3, 3), (4, 4)),
            ]),
            CRATE_A
        ),
        failed(
            "a is not covered 100% by its own unit tests: /repo/crates/ab/src/lib.rs is not a file of a"
        ),
        "the report of another crate, even a fully covered one"
    );
}

#[test]
fn coverage_gate_refuses_an_empty_report() {
    assert_eq!(
        coverage_gate(&report(&[]), CRATE_A),
        failed("a is not covered 100% by its own unit tests: no file of a measured"),
        "a report of nothing proves nothing"
    );
    assert_eq!(
        coverage_gate(
            &report(&[("crates/a/src/lib.rs", (10, 10), (3, 3), (4, 4))]),
            CRATE_A_MAIN
        ),
        failed("the a binary is not covered 100% by its tests: no file of a measured"),
        "a binary report without its main.rs"
    );
}

#[test]
fn rejects_unreadable_reports() {
    assert!(
        coverage_gate("not json", CRATE_A)
            .is_err_and(|error| error.summary.starts_with("invalid coverage report: ")),
        "not JSON"
    );
    assert_eq!(
        coverage_gate(r#"{"data":[{"totals":{}}]}"#, CRATE_A),
        failed("coverage report has no files"),
        "no file list"
    );
    assert_eq!(
        coverage_gate(
            r#"{"data":[{"files":[{"filename":"/r/crates/a/src/lib.rs","summary":{"lines":{"count":1,"covered":1}}}]}]}"#,
            CRATE_A
        ),
        failed("coverage report has no functions total"),
        "a missing total"
    );
}

#[test]
fn requires_the_gate_to_refuse_a_path_only_an_integration_test_reaches() {
    let locality = Coverage {
        krate: crate::COVERAGE_LOCALITY,
        measured: Measured::Library,
    };
    let step: Vec<Step> = check_steps(CHANGES_MODE)
        .into_iter()
        .filter(|step| step.action == Action::CoverageLocality)
        .collect();
    let mut refused = passing();
    assert_eq!(
        run_steps(&step, &mut refused, &mut Vec::new()),
        Ok(()),
        "the gate refuses the fixture"
    );
    assert!(
        refused.ran.contains(&locality.command().line()),
        "the fixture is measured"
    );
    let mut accepted = passing();
    let _full = accepted
        .files
        .insert(locality.report(), full_report(locality));
    assert_eq!(
        run_steps(&step, &mut accepted, &mut Vec::new()),
        failed(format!(
            "Rust coverage counts a crate's own unit tests only: the coverage gate accepted {}: \
             a path reached only by an integration test counts as covered",
            crate::COVERAGE_LOCALITY.dir
        )),
        "a gate that counts integration tests"
    );
    let mut elsewhere = passing();
    let _empty = elsewhere.files.insert(locality.report(), report(&[]));
    assert_eq!(
        run_steps(&step, &mut elsewhere, &mut Vec::new()),
        failed(format!(
            "Rust coverage counts a crate's own unit tests only: the coverage gate refused {} \
             for another reason: coverage-locality is not covered 100% by its own unit tests: \
             no file of coverage-locality measured",
            crate::COVERAGE_LOCALITY.dir
        )),
        "a refusal that proves nothing"
    );
}

#[test]
fn fails_on_missing_or_incomplete_coverage() {
    let core = Coverage {
        krate: crate::RUST_CRATES[0],
        measured: Measured::Library,
    };
    let mut missing = passing();
    let _removed = missing.files.remove(&core.report());
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut missing, &mut Vec::new()),
        failed(
            "Rust coverage of audio-core: 100% by its own unit tests: cannot read \
             target/coverage/audio-core.json: missing"
        ),
        "missing report"
    );
    let mut partial = passing();
    let _replaced = partial.files.insert(
        core.report(),
        report(&[("crates/audio-core/src/lib.rs", (2, 1), (1, 1), (1, 1))]),
    );
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut partial, &mut Vec::new()),
        failed(
            "Rust coverage of audio-core: 100% by its own unit tests: audio-core is not \
             covered 100% by its own unit tests: crates/audio-core/src/lib.rs: lines 1/2\n\
             the lines: cargo +nightly-2026-11-01 llvm-cov --manifest-path \
             crates/audio-core/Cargo.toml --lib --branch --show-missing-lines -- --include-ignored"
        ),
        "incomplete coverage"
    );
    let mut broken = passing();
    broken.fail = Some(core.command().line());
    assert!(
        run_steps(&check_steps(CHANGES_MODE), &mut broken, &mut Vec::new())
            .is_err_and(|failure| failure.summary.ends_with("` exited with code 3")),
        "a measurement that fails"
    );
}

#[test]
fn accepts_only_fully_covered_typescript_packages() {
    assert_eq!(ts_coverage_gate("p", TS_COVERED), Ok(()), "100%");
    assert_eq!(
        ts_coverage_gate(
            "p",
            r#"{"total":{"lines":{"total":4,"covered":3},"statements":{"total":0,"covered":0},"functions":{"total":2},"branches":{"total":3,"covered":3}}}"#
        ),
        Err(
            "p: coverage is not 100%: lines 3/4, statements not measured, no functions total"
                .to_owned()
        ),
        "every kind is checked"
    );
    assert_eq!(
        ts_coverage_gate(
            "p",
            r#"{"total":{"lines":{"total":1,"covered":1},"statements":{"total":1,"covered":1},"functions":{"total":1,"covered":1},"branches":{"total":0,"covered":0}}}"#
        ),
        Ok(()),
        "code without a branch, every line covered"
    );
    assert_eq!(
        ts_coverage_gate(
            "p",
            r#"{"total":{"lines":{"total":0,"covered":0},"statements":{"total":0,"covered":0},"functions":{"total":0,"covered":0},"branches":{"total":0,"covered":0}}}"#
        ),
        Err("p: coverage is not 100%: lines not measured, statements not measured, functions not measured".to_owned()),
        "a report that measured nothing"
    );
    assert_eq!(
        ts_coverage_gate(
            "p",
            r#"{"total":{"lines":{"total":1,"covered":1},"statements":{"total":1,"covered":1},"functions":{"total":1,"covered":1}}}"#
        ),
        Err("p: coverage is not 100%: no branches total".to_owned()),
        "branches absent, not zero"
    );
    assert!(
        ts_coverage_gate("p", "nope")
            .is_err_and(|error| error.starts_with("p: invalid coverage summary: ")),
        "unreadable"
    );
}

#[test]
fn refuses_checks_that_change_the_working_tree() {
    let tree = |status: &str, diff: &str| crate::TreeState {
        status: status.to_owned(),
        diff: diff.to_owned(),
        head: TREE_HEAD.1.to_owned(),
    };
    let dirty = tree(" M a.rs\n?? notes.txt\n", "diff a.rs\n");
    assert_eq!(
        crate::tree_gate(Some(&dirty), &dirty.clone()),
        Ok(()),
        "a dirty tree the checks left alone"
    );
    assert_eq!(
        crate::tree_gate(
            Some(&dirty),
            &tree(" M a.rs\n M Cargo.lock\n", "diff a.rs\n")
        ),
        found(
            "the checks changed the working tree",
            &[
                "` M Cargo.lock` appeared".to_owned(),
                "`?? notes.txt` went away".to_owned()
            ]
        ),
        "a rewritten lock file, a file removed"
    );
    assert_eq!(
        crate::tree_gate(
            Some(&dirty),
            &tree(" M a.rs\n?? notes.txt\n", "other diff\n")
        ),
        failed(
            "the checks changed the working tree: the uncommitted changes of a tracked file changed"
        ),
        "a dirty file changed again"
    );
    for head in [
        "def4560000000000000000000000000000000000\nrefs/heads/main\n",
        "abc1230000000000000000000000000000000000\nrefs/heads/other\n",
        "abc1230000000000000000000000000000000000\nHEAD\n",
    ] {
        let moved = crate::TreeState {
            head: head.to_owned(),
            ..dirty.clone()
        };
        assert_eq!(
            crate::tree_gate(Some(&dirty), &moved),
            failed(format!(
                "the checks changed the working tree: what is checked out changed, from \
                 `abc1230000000000000000000000000000000000 refs/heads/main` to `{}`: the checks read another commit than the one \
                 recorded",
                head.split_whitespace().collect::<Vec<_>>().join(" ")
            )),
            "a commit made, a branch checked out or HEAD detached during the checks: {head:?}"
        );
    }
    assert_eq!(
        crate::tree_gate(None, &dirty),
        failed("the working tree was not recorded before the checks"),
        "nothing to compare with"
    );
    let mut committed = passing();
    committed.rewrite = Some((
        "cargo deny --locked check".to_owned(),
        format!("capture {}", TREE_HEAD.0),
        "def4560000000000000000000000000000000000\nrefs/heads/main\n".to_owned(),
    ));
    assert!(
        run_steps(&check_steps(CHANGES_MODE), &mut committed, &mut Vec::new()).is_err_and(
            |failure| failure
                .summary
                .contains("what is checked out changed, from `abc1230000000000000000000000000000000000 refs/heads/main` to `def4560000000000000000000000000000000000 refs/heads/main`")
        ),
        "a commit made in the same checkout while the check runs fails it"
    );
    let mut rewritten = passing();
    rewritten.rewrite = Some((
        "cargo deny --locked check".to_owned(),
        format!("capture {}", TREE_STATUS.0),
        "?? notes.txt\n M Cargo.lock\n".to_owned(),
    ));
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut rewritten, &mut Vec::new()),
        failed(
            "Working tree unchanged by the checks: the checks changed the working tree: \
             ` M Cargo.lock` appeared"
        ),
        "a command that rewrites a tracked file fails the whole check, at its end"
    );
    let last = check_steps(CHANGES_MODE).split_off(check_steps(CHANGES_MODE).len() - 1);
    assert_eq!(
        run_steps(&last, &mut passing(), &mut Vec::new()),
        failed(
            "Working tree unchanged by the checks: the working tree was not recorded before \
             the checks"
        ),
        "the comparison alone"
    );
}

#[test]
fn reads_the_changes_as_they_are() {
    let repo = scratch("textconv");
    let _init = hermetic_git(&repo, &["init", "--quiet"]);
    std::fs::write(repo.join("a.rs"), "fn f() {}\n").expect("source");
    let _add = hermetic_git(&repo, &["add", "a.rs"]);
    let _commit = hermetic_git(&repo, &["commit", "--quiet", "-m", "x"]);
    std::fs::write(repo.join("a.rs"), "fn g() {}\n").expect("changed");
    std::fs::write(repo.join(".gitattributes"), "*.rs diff=upper\n").expect("attributes");
    for (key, value) in [
        ("diff.upper.textconv", "tr a-z A-Z <"),
        ("diff.external", "false"),
    ] {
        let _set = hermetic_git(&repo, &["config", key, value]);
    }
    let mut runner = system(repo.path());
    let state = crate::coverage::tree_state(&mut runner);
    let diff = state.expect("read").diff;
    assert!(
        diff.contains("-fn f() {}\n+fn g() {}\n"),
        "neither an external diff nor a text conversion: {diff}"
    );
}
